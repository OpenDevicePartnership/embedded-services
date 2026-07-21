#![allow(clippy::unwrap_used)]
#![allow(clippy::panic)]
use std::ptr;

use embassy_futures::join::join;
use embassy_time::{Duration, Instant, TimeoutError, with_timeout};
use embedded_usb_pd::{
    PowerRole,
    constants::{T_PS_TRANSITION_EPR_MS, T_PS_TRANSITION_SPR_MS},
    type_c::ConnectionState,
};
use power_policy_interface::{
    capability::{
        ConsumerDisconnect, ConsumerFlags, ConsumerPowerCapability, ProviderFlags, ProviderPowerCapability, PsuType,
    },
    psu::{Psu, PsuState},
    service::event::Event as PowerPolicyEvent,
};
use type_c_interface::{
    control::pd::PortStatus,
    port::event::{PortEvent, PortEventBitfield, PortStatusEventBitfield},
    port::max_sink_voltage::MaxSinkVoltage,
    util::POWER_CAPABILITY_5V_1A5,
};
use type_c_interface_test_mocks::controller::{
    FnCall as ControllerFnCall, max_sink_voltage::FnCall as MaxSinkVoltageFnCall, pd::FnCall as PdFnCall,
};
use type_c_service::controller::event::Event;

use crate::common::{
    DEFAULT_PER_CALL_TIMEOUT, DEFAULT_TEST_DURATION, PowerPolicyServiceReceiver, Test, TestPort, TypeCServiceReceiver,
};

mod common;

/// Test basic consumer attach flow
struct TestBasicConsumerFlow;

impl Test for TestBasicConsumerFlow {
    async fn run<'port, 'ch>(
        &mut self,
        type_c_receiver: TypeCServiceReceiver<'port, 'ch>,
        power_policy_receiver: PowerPolicyServiceReceiver<'port, 'ch>,
        port0: TestPort<'port, 'ch>,
        _port1: TestPort<'port, 'ch>,
        _port2: TestPort<'port, 'ch>,
    ) {
        {
            // Set up the mock to report a sink connection and allow enabling the sink path
            let mut mock0 = port0.mock.lock().await;

            mock0.next_result_get_port_status.push_back(Ok(PortStatus {
                available_sink_contract: Some(POWER_CAPABILITY_5V_1A5),
                connection_state: Some(ConnectionState::Attached),
                power_role: PowerRole::Sink,
                ..Default::default()
            }));
            mock0.next_result_enable_sink_path.push_back(Ok(()));
        }

        // Simulate a plug event and a new consumer contract
        let mut port_event = PortStatusEventBitfield::none();
        port_event.set_plug_inserted_or_removed(true);
        port_event.set_new_power_contract_as_consumer(true);
        port_event.set_sink_ready(true);

        port0
            .port
            .lock()
            .await
            .process_event(Event::PortEvent(PortEvent::StatusChanged(port_event)))
            .await
            .unwrap();

        let (type_c_result, power_policy_result) = join(
            with_timeout(DEFAULT_PER_CALL_TIMEOUT, type_c_receiver.receive()),
            with_timeout(DEFAULT_PER_CALL_TIMEOUT, power_policy_receiver.receive()),
        )
        .await;

        // Shouldn't get any Type-C service events in this flow
        assert_eq!(type_c_result.err(), Some(TimeoutError));

        // Power policy service should broadcast a consumer connect event
        match power_policy_result {
            Ok(PowerPolicyEvent::ConsumerConnected(psu, capability)) => {
                assert_eq!(
                    capability,
                    ConsumerPowerCapability {
                        capability: POWER_CAPABILITY_5V_1A5,
                        flags: ConsumerFlags::none().with_psu_type(PsuType::TypeC),
                    }
                );
                assert!(ptr::eq(psu, port0.port));
            }
            _ => panic!("Did not receive consumer connected event"),
        }

        // The port should now be tracking a connected consumer internally
        assert!(matches!(
            port0.port.lock().await.state().psu_state,
            PsuState::ConnectedConsumer(_)
        ));

        {
            // Set up the mock to report an unplug
            let mut mock0 = port0.mock.lock().await;
            let port_status = Ok(Default::default());
            mock0.next_result_get_port_status.push_back(port_status);
        }

        // Simulate an unplug event
        let mut port_event = PortStatusEventBitfield::none();
        port_event.set_plug_inserted_or_removed(true);

        port0
            .port
            .lock()
            .await
            .process_event(Event::PortEvent(PortEvent::StatusChanged(port_event)))
            .await
            .unwrap();

        let (type_c_result, power_policy_result) = join(
            with_timeout(DEFAULT_PER_CALL_TIMEOUT, type_c_receiver.receive()),
            with_timeout(DEFAULT_PER_CALL_TIMEOUT, power_policy_receiver.receive()),
        )
        .await;

        // Type-C service currently shouldn't broadcast any events in this flow
        assert_eq!(type_c_result.err(), Some(TimeoutError));
        // Power policy service should broadcast a consumer disconnect event
        match power_policy_result {
            Ok(PowerPolicyEvent::ConsumerDisconnected(psu, _)) => {
                assert!(ptr::eq(psu, port0.port));
            }
            _ => panic!("Did not receive consumer disconnected event"),
        }

        // The port should be detached again after unplug
        assert_eq!(port0.port.lock().await.state().psu_state, PsuState::Detached);
    }
}

/// Test basic provider attach flow: plug -> new provider contract -> unplug.
///
/// Validates the internal `psu_state` transitions (`Detached` -> `ConnectedProvider` -> `Detached`)
/// and that the matching `ProviderConnected`/`ProviderDisconnected` events are broadcast to the
/// power policy service.
struct TestBasicProviderFlow;

impl Test for TestBasicProviderFlow {
    async fn run<'port, 'ch>(
        &mut self,
        type_c_receiver: TypeCServiceReceiver<'port, 'ch>,
        power_policy_receiver: PowerPolicyServiceReceiver<'port, 'ch>,
        port0: TestPort<'port, 'ch>,
        _port1: TestPort<'port, 'ch>,
        _port2: TestPort<'port, 'ch>,
    ) {
        // The port should start out detached.
        assert_eq!(port0.port.lock().await.state().psu_state, PsuState::Detached);

        {
            // Set up the mock to report a source connection
            let mut mock0 = port0.mock.lock().await;

            mock0.next_result_get_port_status.push_back(Ok(PortStatus {
                available_source_contract: Some(POWER_CAPABILITY_5V_1A5),
                connection_state: Some(ConnectionState::Attached),
                power_role: PowerRole::Source,
                ..Default::default()
            }));
        }

        // Simulate a plug event and a new provider contract
        let mut port_event = PortStatusEventBitfield::none();
        port_event.set_plug_inserted_or_removed(true);
        port_event.set_new_power_contract_as_provider(true);

        port0
            .port
            .lock()
            .await
            .process_event(Event::PortEvent(PortEvent::StatusChanged(port_event)))
            .await
            .unwrap();

        let (type_c_result, power_policy_result) = join(
            with_timeout(DEFAULT_PER_CALL_TIMEOUT, type_c_receiver.receive()),
            with_timeout(DEFAULT_PER_CALL_TIMEOUT, power_policy_receiver.receive()),
        )
        .await;

        // Shouldn't get any Type-C service events in this flow
        assert_eq!(type_c_result.err(), Some(TimeoutError));

        // Power policy service should broadcast a provider connect event
        match power_policy_result {
            Ok(PowerPolicyEvent::ProviderConnected(psu, capability)) => {
                assert_eq!(
                    capability,
                    ProviderPowerCapability {
                        capability: POWER_CAPABILITY_5V_1A5,
                        flags: ProviderFlags::none().with_psu_type(PsuType::TypeC),
                    }
                );
                assert!(ptr::eq(psu, port0.port));
            }
            _ => panic!("Did not receive provider connected event"),
        }

        // The port should now be tracking a connected provider internally
        assert!(matches!(
            port0.port.lock().await.state().psu_state,
            PsuState::ConnectedProvider(_)
        ));

        {
            // Set up the mock to report an unplug
            let mut mock0 = port0.mock.lock().await;
            let port_status = Ok(Default::default());
            mock0.next_result_get_port_status.push_back(port_status);
        }

        // Simulate an unplug event
        let mut port_event = PortStatusEventBitfield::none();
        port_event.set_plug_inserted_or_removed(true);

        port0
            .port
            .lock()
            .await
            .process_event(Event::PortEvent(PortEvent::StatusChanged(port_event)))
            .await
            .unwrap();

        let (type_c_result, power_policy_result) = join(
            with_timeout(DEFAULT_PER_CALL_TIMEOUT, type_c_receiver.receive()),
            with_timeout(DEFAULT_PER_CALL_TIMEOUT, power_policy_receiver.receive()),
        )
        .await;

        // Type-C service currently shouldn't broadcast any events in this flow
        assert_eq!(type_c_result.err(), Some(TimeoutError));
        // Power policy service should broadcast a provider disconnect event
        match power_policy_result {
            Ok(PowerPolicyEvent::ProviderDisconnected(psu)) => {
                assert!(ptr::eq(psu, port0.port));
            }
            _ => panic!("Did not receive provider disconnected event"),
        }

        // The port should be detached again after unplug
        assert_eq!(port0.port.lock().await.state().psu_state, PsuState::Detached);
    }
}

/// End-to-end test of the software sink-ready timeout that drives the real `EventReceiver` and
/// exercises every internal state transition along with the power-policy broadcasts.
///
/// The controller never raises a hardware sink-ready event, so a real `embassy_time::Timer` inside
/// a live [`type_c_service::controller::event_receiver::EventReceiver`] must elapse and synthesize
/// the sink-ready event that completes the consumer contract. The event receiver is driven
/// manually, one event at a time, so the port's internal state can be asserted deterministically
/// between transitions:
///
/// * `Detached` with no armed timeout initially,
/// * `Idle` with the sink-ready timeout armed after the plug (no consumer broadcast yet),
/// * `ConnectedConsumer` with the timeout cleared once the timer fires (`ConsumerConnected`),
/// * `Detached` with no armed timeout after the unplug (`ConsumerDisconnected`).
struct TestConsumerFlowTimerSinkReady;

impl Test for TestConsumerFlowTimerSinkReady {
    async fn run<'port, 'ch>(
        &mut self,
        _type_c_receiver: TypeCServiceReceiver<'port, 'ch>,
        power_policy_receiver: PowerPolicyServiceReceiver<'port, 'ch>,
        port0: TestPort<'port, 'ch>,
        _port1: TestPort<'port, 'ch>,
        _port2: TestPort<'port, 'ch>,
    ) {
        let TestPort {
            port,
            mock,
            shared_state,
            interrupt_sender,
            mut event_receiver,
        } = port0;

        {
            // Queue the controller's status responses in call order. No hardware sink-ready event
            // is ever raised, so the sink-ready poll below is driven entirely by the software timer.
            let mut mock0 = mock.lock().await;
            // Plug: report a connected sink so the port begins the consumer-attach flow.
            mock0.next_result_get_port_status.push_back(Ok(PortStatus {
                available_sink_contract: Some(POWER_CAPABILITY_5V_1A5),
                connection_state: Some(ConnectionState::Attached),
                power_role: PowerRole::Sink,
                ..Default::default()
            }));
            // Timer-driven sink-ready poll: still a connected sink, which completes the contract.
            mock0.next_result_get_port_status.push_back(Ok(PortStatus {
                available_sink_contract: Some(POWER_CAPABILITY_5V_1A5),
                connection_state: Some(ConnectionState::Attached),
                power_role: PowerRole::Sink,
                ..Default::default()
            }));
            // Unplug: report a detached/default status so the consumer disconnects.
            mock0.next_result_get_port_status.push_back(Ok(Default::default()));
            // Sink path is enabled when the power policy connects the consumer.
            mock0.next_result_enable_sink_path.push_back(Ok(()));
        }

        // Initially detached with no pending sink-ready timeout.
        assert_eq!(port.lock().await.state().psu_state, PsuState::Detached);
        assert!(shared_state.lock().await.sink_ready_deadline().is_none());

        let start = Instant::now();

        // Plug in with a new consumer contract but WITHOUT a hardware sink-ready event.
        let mut interrupt = PortEventBitfield::none();
        interrupt.status.set_plug_inserted_or_removed(true);
        interrupt.status.set_new_power_contract_as_consumer(true);
        interrupt_sender.send(interrupt).await;

        // Drive the receiver manually so the intermediate state is observable before the timer
        // fires. This first event is the plug interrupt that was just sent.
        let event = event_receiver.wait_event().await;
        port.lock().await.process_event(event).await.unwrap();

        // The port is attached but not consuming yet, the sink-ready timeout is armed, and no
        // consumer connection has been broadcast to the power policy.
        assert_eq!(port.lock().await.state().psu_state, PsuState::Idle);
        assert!(shared_state.lock().await.sink_ready_deadline().is_some());
        assert!(power_policy_receiver.try_receive().is_err());

        // The next event is synthesized *inside* `wait_event` by a real timer; nothing in this test
        // injects a sink-ready event. This call blocks until that timer elapses.
        let event = event_receiver.wait_event().await;
        let elapsed = start.elapsed();
        port.lock().await.process_event(event).await.unwrap();

        // The connect must have waited for the sink-ready timer to elapse, proving it was
        // timer-driven rather than an immediate hardware sink-ready event.
        assert!(
            elapsed >= Duration::from_millis(T_PS_TRANSITION_SPR_MS.maximum.0 as u64),
            "consumer connected before the sink-ready timer could elapse: {}ms",
            elapsed.as_millis()
        );

        // The timer cleared the sink-ready timeout when it synthesized the sink-ready event. The
        // port is not a connected consumer yet: it has only forwarded the updated consumer
        // capability to the power policy, which still has to connect it.
        assert!(shared_state.lock().await.sink_ready_deadline().is_none());

        // The power policy should now broadcast a consumer connect event.
        match with_timeout(DEFAULT_PER_CALL_TIMEOUT, power_policy_receiver.receive()).await {
            Ok(PowerPolicyEvent::ConsumerConnected(psu, capability)) => {
                assert_eq!(
                    capability,
                    ConsumerPowerCapability {
                        capability: POWER_CAPABILITY_5V_1A5,
                        flags: ConsumerFlags::none().with_psu_type(PsuType::TypeC),
                    }
                );
                assert!(ptr::eq(psu, port));
            }
            _ => panic!("Did not receive consumer connected event from software sink-ready timeout"),
        }

        // Connecting the consumer is what moves the port into the connected-consumer state.
        assert!(matches!(
            port.lock().await.state().psu_state,
            PsuState::ConnectedConsumer(_)
        ));

        // Unplug.
        let mut interrupt = PortEventBitfield::none();
        interrupt.status.set_plug_inserted_or_removed(true);
        interrupt_sender.send(interrupt).await;

        // Process the unplug event.
        let event = event_receiver.wait_event().await;
        port.lock().await.process_event(event).await.unwrap();

        // The power policy should broadcast a consumer disconnect event.
        match with_timeout(DEFAULT_PER_CALL_TIMEOUT, power_policy_receiver.receive()).await {
            Ok(PowerPolicyEvent::ConsumerDisconnected(psu, _)) => {
                assert!(ptr::eq(psu, port));
            }
            _ => panic!("Did not receive consumer disconnected event"),
        }

        // Back to detached with no pending sink-ready timeout.
        assert_eq!(port.lock().await.state().psu_state, PsuState::Detached);
        assert!(shared_state.lock().await.sink_ready_deadline().is_none());
    }
}

/// Test that changing the max sink voltage while a consumer is connected disables the sink path and
/// notifies the power policy, which broadcasts a `ConsumerDisconnected` event with the renegotiation
/// flag set. Setting the same voltage should do neither.
struct TestSinkDisableOnVoltageChange;

impl Test for TestSinkDisableOnVoltageChange {
    async fn run<'port, 'ch>(
        &mut self,
        type_c_receiver: TypeCServiceReceiver<'port, 'ch>,
        power_policy_receiver: PowerPolicyServiceReceiver<'port, 'ch>,
        port0: TestPort<'port, 'ch>,
        _port1: TestPort<'port, 'ch>,
        _port2: TestPort<'port, 'ch>,
    ) {
        // Bring up a connected consumer at 5V.
        {
            let mut mock0 = port0.mock.lock().await;
            mock0.next_result_get_port_status.push_back(Ok(PortStatus {
                available_sink_contract: Some(POWER_CAPABILITY_5V_1A5),
                connection_state: Some(ConnectionState::Attached),
                power_role: PowerRole::Sink,
                ..Default::default()
            }));
            // Sink path is enabled when the power policy connects the consumer.
            mock0.next_result_enable_sink_path.push_back(Ok(()));
        }

        let mut port_event = PortStatusEventBitfield::none();
        port_event.set_plug_inserted_or_removed(true);
        port_event.set_new_power_contract_as_consumer(true);
        port_event.set_sink_ready(true);
        port0
            .port
            .lock()
            .await
            .process_event(Event::PortEvent(PortEvent::StatusChanged(port_event)))
            .await
            .unwrap();

        // Wait for the power policy to connect the consumer so the port is in the connected state.
        let (_type_c_result, power_policy_result) = join(
            with_timeout(DEFAULT_PER_CALL_TIMEOUT, type_c_receiver.receive()),
            with_timeout(DEFAULT_PER_CALL_TIMEOUT, power_policy_receiver.receive()),
        )
        .await;

        match power_policy_result {
            Ok(PowerPolicyEvent::ConsumerConnected(psu, _)) => assert!(ptr::eq(psu, port0.port)),
            _ => panic!("Did not receive consumer connected event"),
        }

        // Setting the same voltage as the active contract must not disable the sink path or disconnect.
        {
            let mut mock0 = port0.mock.lock().await;
            mock0.fn_calls.clear();
            mock0.next_result_set_max_sink_voltage.push_back(Ok(()));
        }
        port0.port.lock().await.set_max_sink_voltage(Some(5000)).await.unwrap();
        {
            let mut mock0 = port0.mock.lock().await;
            assert!(
                matches!(
                    mock0.fn_calls.pop_front(),
                    Some(ControllerFnCall::MaxSinkVoltage(
                        MaxSinkVoltageFnCall::SetMaxSinkVoltage(_, Some(5000))
                    ))
                ),
                "expected only the max sink voltage to be set without disabling the sink path"
            );
            assert!(mock0.fn_calls.is_empty());
        }
        // No disconnect should have been broadcast.
        assert!(matches!(
            with_timeout(DEFAULT_PER_CALL_TIMEOUT, power_policy_receiver.receive()).await,
            Err(TimeoutError)
        ));

        // Changing the max sink voltage should disable the sink path and notify the power policy.
        {
            let mut mock0 = port0.mock.lock().await;
            mock0.fn_calls.clear();
            mock0.next_result_enable_sink_path.push_back(Ok(()));
            mock0.next_result_set_max_sink_voltage.push_back(Ok(()));
        }
        port0.port.lock().await.set_max_sink_voltage(Some(9000)).await.unwrap();

        // The power policy should broadcast a consumer disconnect with the renegotiation flag set.
        match with_timeout(DEFAULT_PER_CALL_TIMEOUT, power_policy_receiver.receive()).await {
            Ok(PowerPolicyEvent::ConsumerDisconnected(psu, flags)) => {
                assert!(ptr::eq(psu, port0.port));
                assert_eq!(flags, ConsumerDisconnect::none().with_renegotiation(true));
            }
            _ => panic!("Did not receive consumer disconnected event"),
        }

        // The sink path should have been disabled before the new voltage was applied.
        {
            let mut mock0 = port0.mock.lock().await;
            assert!(
                matches!(
                    mock0.fn_calls.pop_front(),
                    Some(ControllerFnCall::Pd(PdFnCall::EnableSinkPath(_, false)))
                ),
                "expected the sink path to be disabled before the voltage change"
            );
            assert!(
                matches!(
                    mock0.fn_calls.pop_front(),
                    Some(ControllerFnCall::MaxSinkVoltage(
                        MaxSinkVoltageFnCall::SetMaxSinkVoltage(_, Some(9000))
                    ))
                ),
                "expected the max sink voltage to be set"
            );
            assert!(mock0.fn_calls.is_empty());
        }
    }
}

/// It's not possible to know if setting the max sink voltage will trigger a renegotiation
/// because the logic to select a particular contract is specific to the PD controller.
/// This test ensures that the sink path is disabled and the power policy is notified regardless of whether a renegotiation occurs.
struct TestSetMaxSinkVoltageRecovery;

impl Test for TestSetMaxSinkVoltageRecovery {
    async fn run<'port, 'ch>(
        &mut self,
        type_c_receiver: TypeCServiceReceiver<'port, 'ch>,
        power_policy_receiver: PowerPolicyServiceReceiver<'port, 'ch>,
        port0: TestPort<'port, 'ch>,
        _port1: TestPort<'port, 'ch>,
        _port2: TestPort<'port, 'ch>,
    ) {
        let TestPort {
            port,
            mock,
            mut event_receiver,
            ..
        } = port0;

        {
            // Set up the mock to report a sink connection and allow enabling the sink path
            let mut mock = mock.lock().await;

            mock.next_result_get_port_status.push_back(Ok(PortStatus {
                available_sink_contract: Some(POWER_CAPABILITY_5V_1A5),
                connection_state: Some(ConnectionState::Attached),
                power_role: PowerRole::Sink,
                ..Default::default()
            }));
            mock.next_result_enable_sink_path.push_back(Ok(()));
        }

        // Simulate a plug event and a new consumer contract
        let mut port_event = PortStatusEventBitfield::none();
        port_event.set_plug_inserted_or_removed(true);
        port_event.set_new_power_contract_as_consumer(true);
        port_event.set_sink_ready(true);

        port.lock()
            .await
            .process_event(Event::PortEvent(PortEvent::StatusChanged(port_event)))
            .await
            .unwrap();

        let (type_c_result, power_policy_result) = join(
            with_timeout(DEFAULT_PER_CALL_TIMEOUT, type_c_receiver.receive()),
            with_timeout(DEFAULT_PER_CALL_TIMEOUT, power_policy_receiver.receive()),
        )
        .await;

        // Power policy service should broadcast a consumer connected event
        match power_policy_result {
            Ok(PowerPolicyEvent::ConsumerConnected(psu, capability)) => {
                assert_eq!(
                    capability,
                    ConsumerPowerCapability {
                        capability: POWER_CAPABILITY_5V_1A5,
                        flags: ConsumerFlags::none().with_psu_type(PsuType::TypeC),
                    }
                );
                assert!(ptr::eq(psu, port0.port));
            }
            _ => panic!("Did not receive consumer connected event"),
        }
        // Shouldn't get any Type-C service events in this flow
        assert_eq!(type_c_result.err(), Some(TimeoutError));

        {
            // Set up the mock to accept a max sink voltage change and disable the sink path
            let mut mock = mock.lock().await;

            mock.next_result_set_max_sink_voltage.push_back(Ok(()));
            mock.next_result_enable_sink_path.push_back(Ok(()));
        }

        port.lock().await.set_max_sink_voltage(None).await.unwrap();

        let (type_c_result, power_policy_result) = join(
            with_timeout(DEFAULT_PER_CALL_TIMEOUT, type_c_receiver.receive()),
            with_timeout(DEFAULT_PER_CALL_TIMEOUT, power_policy_receiver.receive()),
        )
        .await;

        // Power policy service should broadcast a consumer disconnected event
        match power_policy_result {
            Ok(PowerPolicyEvent::ConsumerDisconnected(psu, flags)) => {
                assert_eq!(flags, ConsumerDisconnect::none().with_renegotiation(true));
                assert!(ptr::eq(psu, port0.port));
            }
            _ => panic!("Did not receive provider connected event"),
        }
        // Shouldn't get any Type-C service events in this flow
        assert_eq!(type_c_result.err(), Some(TimeoutError));

        {
            // Setup the mock to return the existing contract and enable the sink path again
            let mut mock = mock.lock().await;

            mock.next_result_get_port_status.push_back(Ok(PortStatus {
                available_sink_contract: Some(POWER_CAPABILITY_5V_1A5),
                connection_state: Some(ConnectionState::Attached),
                power_role: PowerRole::Sink,
                ..Default::default()
            }));
            mock.next_result_enable_sink_path.push_back(Ok(()));
        }

        // Wait for sink ready recovery, x3 just to be safe and ensure the deadline has passed.
        let event = with_timeout(
            Duration::from_millis(T_PS_TRANSITION_EPR_MS.maximum.0 as u64 * 3),
            event_receiver.wait_event(),
        )
        .await
        .unwrap();

        port.lock().await.process_event(event).await.unwrap();

        let (type_c_result, power_policy_result) = join(
            with_timeout(DEFAULT_PER_CALL_TIMEOUT, type_c_receiver.receive()),
            with_timeout(DEFAULT_PER_CALL_TIMEOUT, power_policy_receiver.receive()),
        )
        .await;

        // Power policy service should broadcast a consumer connected event
        match power_policy_result {
            Ok(PowerPolicyEvent::ConsumerConnected(psu, capability)) => {
                assert_eq!(
                    capability,
                    ConsumerPowerCapability {
                        capability: POWER_CAPABILITY_5V_1A5,
                        flags: ConsumerFlags::none().with_psu_type(PsuType::TypeC),
                    }
                );
                assert!(ptr::eq(psu, port0.port));
            }
            _ => panic!("Did not receive consumer connected event"),
        }
        // Shouldn't get any Type-C service events in this flow
        assert_eq!(type_c_result.err(), Some(TimeoutError));
    }
}

/// Test a power role swap from consumer to provider.
///
/// Starting from a connected consumer, a power role swap turns the port into a provider. The port
/// should tear down the consumer contract and stand up the new provider contract, so the power
/// policy service must broadcast a `ConsumerDisconnected` event followed by a `ProviderConnected`
/// event carrying the negotiated provider capability. The internal `psu_state` must end up in
/// `ConnectedProvider`.
struct TestConsumerToProviderRoleSwap;

impl Test for TestConsumerToProviderRoleSwap {
    async fn run<'port, 'ch>(
        &mut self,
        _type_c_receiver: TypeCServiceReceiver<'port, 'ch>,
        power_policy_receiver: PowerPolicyServiceReceiver<'port, 'ch>,
        port0: TestPort<'port, 'ch>,
        _port1: TestPort<'port, 'ch>,
        _port2: TestPort<'port, 'ch>,
    ) {
        // The port should start out detached.
        assert_eq!(port0.port.lock().await.state().psu_state, PsuState::Detached);

        // Bring up a connected consumer at 5V.
        {
            let mut mock0 = port0.mock.lock().await;
            mock0.next_result_get_port_status.push_back(Ok(PortStatus {
                available_sink_contract: Some(POWER_CAPABILITY_5V_1A5),
                connection_state: Some(ConnectionState::Attached),
                power_role: PowerRole::Sink,
                ..Default::default()
            }));
            // Sink path is enabled when the power policy connects the consumer.
            mock0.next_result_enable_sink_path.push_back(Ok(()));
        }

        let mut port_event = PortStatusEventBitfield::none();
        port_event.set_plug_inserted_or_removed(true);
        port_event.set_new_power_contract_as_consumer(true);
        port_event.set_sink_ready(true);
        port0
            .port
            .lock()
            .await
            .process_event(Event::PortEvent(PortEvent::StatusChanged(port_event)))
            .await
            .unwrap();

        // The power policy should connect the consumer first.
        match with_timeout(DEFAULT_PER_CALL_TIMEOUT, power_policy_receiver.receive()).await {
            Ok(PowerPolicyEvent::ConsumerConnected(psu, capability)) => {
                assert_eq!(
                    capability,
                    ConsumerPowerCapability {
                        capability: POWER_CAPABILITY_5V_1A5,
                        flags: ConsumerFlags::none().with_psu_type(PsuType::TypeC),
                    }
                );
                assert!(ptr::eq(psu, port0.port));
            }
            _ => panic!("Did not receive consumer connected event"),
        }
        assert!(matches!(
            port0.port.lock().await.state().psu_state,
            PsuState::ConnectedConsumer(_)
        ));

        // Send the power swap completed event.
        {
            let mut mock0 = port0.mock.lock().await;
            mock0.next_result_get_port_status.push_back(Ok(PortStatus {
                connection_state: Some(ConnectionState::Attached),
                power_role: PowerRole::Source,
                ..Default::default()
            }));
            // The consumer is torn down as part of the swap, which disables the sink path.
            mock0.next_result_enable_sink_path.push_back(Ok(()));
        }

        let mut port_event = PortStatusEventBitfield::none();
        port_event.set_power_swap_completed(true);
        port0
            .port
            .lock()
            .await
            .process_event(Event::PortEvent(PortEvent::StatusChanged(port_event)))
            .await
            .unwrap();

        // The consumer should disconnect as soon as the swap completes.
        match with_timeout(DEFAULT_PER_CALL_TIMEOUT, power_policy_receiver.receive()).await {
            Ok(PowerPolicyEvent::ConsumerDisconnected(psu, _)) => {
                assert!(ptr::eq(psu, port0.port));
            }
            _ => panic!("Did not receive consumer disconnected event on role swap"),
        }

        // The port is torn down to an idle state between the swap and the new contract.
        assert_eq!(port0.port.lock().await.state().psu_state, PsuState::Idle);

        // The sink path should be disabled as part of tearing down the consumer contract.
        {
            let mock0 = port0.mock.lock().await;
            assert!(
                mock0
                    .fn_calls
                    .iter()
                    .any(|call| matches!(call, ControllerFnCall::Pd(PdFnCall::EnableSinkPath(_, false)))),
                "expected sink path to be disabled on role swap"
            );
        }

        // Event 3: the new provider contract is negotiated.
        {
            let mut mock0 = port0.mock.lock().await;
            mock0.next_result_get_port_status.push_back(Ok(PortStatus {
                available_source_contract: Some(POWER_CAPABILITY_5V_1A5),
                connection_state: Some(ConnectionState::Attached),
                power_role: PowerRole::Source,
                ..Default::default()
            }));
        }

        let mut port_event = PortStatusEventBitfield::none();
        port_event.set_new_power_contract_as_provider(true);
        port0
            .port
            .lock()
            .await
            .process_event(Event::PortEvent(PortEvent::StatusChanged(port_event)))
            .await
            .unwrap();

        // The power policy should broadcast the new provider contract.
        match with_timeout(DEFAULT_PER_CALL_TIMEOUT, power_policy_receiver.receive()).await {
            Ok(PowerPolicyEvent::ProviderConnected(psu, capability)) => {
                assert_eq!(
                    capability,
                    ProviderPowerCapability {
                        capability: POWER_CAPABILITY_5V_1A5,
                        flags: ProviderFlags::none().with_psu_type(PsuType::TypeC),
                    }
                );
                assert!(ptr::eq(psu, port0.port));
            }
            _ => panic!("Did not receive provider connected event on role swap"),
        }

        // The port should now be tracking a connected provider internally.
        assert!(matches!(
            port0.port.lock().await.state().psu_state,
            PsuState::ConnectedProvider(_)
        ));
    }
}

/// Test a power role swap from provider to consumer.
///
/// Starting from a connected provider, a power role swap turns the port into a consumer. The port
/// should tear down the provider contract and stand up the new consumer contract, so the power
/// policy service must broadcast a `ProviderDisconnected` event followed by a `ConsumerConnected`
/// event carrying the negotiated consumer capability. The internal `psu_state` must end up in
/// `ConnectedConsumer`.
struct TestProviderToConsumerRoleSwap;

impl Test for TestProviderToConsumerRoleSwap {
    async fn run<'port, 'ch>(
        &mut self,
        _type_c_receiver: TypeCServiceReceiver<'port, 'ch>,
        power_policy_receiver: PowerPolicyServiceReceiver<'port, 'ch>,
        port0: TestPort<'port, 'ch>,
        _port1: TestPort<'port, 'ch>,
        _port2: TestPort<'port, 'ch>,
    ) {
        // The port should start out detached.
        assert_eq!(port0.port.lock().await.state().psu_state, PsuState::Detached);

        // Bring up a connected provider at 5V.
        {
            let mut mock0 = port0.mock.lock().await;
            mock0.next_result_get_port_status.push_back(Ok(PortStatus {
                available_source_contract: Some(POWER_CAPABILITY_5V_1A5),
                connection_state: Some(ConnectionState::Attached),
                power_role: PowerRole::Source,
                ..Default::default()
            }));
        }

        let mut port_event = PortStatusEventBitfield::none();
        port_event.set_plug_inserted_or_removed(true);
        port_event.set_new_power_contract_as_provider(true);
        port0
            .port
            .lock()
            .await
            .process_event(Event::PortEvent(PortEvent::StatusChanged(port_event)))
            .await
            .unwrap();

        // The power policy should connect the provider first.
        match with_timeout(DEFAULT_PER_CALL_TIMEOUT, power_policy_receiver.receive()).await {
            Ok(PowerPolicyEvent::ProviderConnected(psu, capability)) => {
                assert_eq!(
                    capability,
                    ProviderPowerCapability {
                        capability: POWER_CAPABILITY_5V_1A5,
                        flags: ProviderFlags::none().with_psu_type(PsuType::TypeC),
                    }
                );
                assert!(ptr::eq(psu, port0.port));
            }
            _ => panic!("Did not receive provider connected event"),
        }
        assert!(matches!(
            port0.port.lock().await.state().psu_state,
            PsuState::ConnectedProvider(_)
        ));

        // Send the power swap completed event.
        {
            let mut mock0 = port0.mock.lock().await;
            mock0.next_result_get_port_status.push_back(Ok(PortStatus {
                connection_state: Some(ConnectionState::Attached),
                power_role: PowerRole::Sink,
                ..Default::default()
            }));
        }

        let mut port_event = PortStatusEventBitfield::none();
        port_event.set_power_swap_completed(true);
        port0
            .port
            .lock()
            .await
            .process_event(Event::PortEvent(PortEvent::StatusChanged(port_event)))
            .await
            .unwrap();

        // The provider should disconnect as soon as the swap completes.
        match with_timeout(DEFAULT_PER_CALL_TIMEOUT, power_policy_receiver.receive()).await {
            Ok(PowerPolicyEvent::ProviderDisconnected(psu)) => {
                assert!(ptr::eq(psu, port0.port));
            }
            _ => panic!("Did not receive provider disconnected event on role swap"),
        }

        // The port is torn down to an idle state between the swap and the new contract.
        assert_eq!(port0.port.lock().await.state().psu_state, PsuState::Idle);

        // The new consumer contract is negotiated.
        {
            let mut mock0 = port0.mock.lock().await;
            mock0.next_result_get_port_status.push_back(Ok(PortStatus {
                available_sink_contract: Some(POWER_CAPABILITY_5V_1A5),
                connection_state: Some(ConnectionState::Attached),
                power_role: PowerRole::Sink,
                ..Default::default()
            }));
            // Sink path is enabled when the power policy connects the new consumer.
            mock0.next_result_enable_sink_path.push_back(Ok(()));
        }

        let mut port_event = PortStatusEventBitfield::none();
        port_event.set_new_power_contract_as_consumer(true);
        port_event.set_sink_ready(true);
        port0
            .port
            .lock()
            .await
            .process_event(Event::PortEvent(PortEvent::StatusChanged(port_event)))
            .await
            .unwrap();

        // Then the power policy should broadcast the new consumer contract.
        match with_timeout(DEFAULT_PER_CALL_TIMEOUT, power_policy_receiver.receive()).await {
            Ok(PowerPolicyEvent::ConsumerConnected(psu, capability)) => {
                assert_eq!(
                    capability,
                    ConsumerPowerCapability {
                        capability: POWER_CAPABILITY_5V_1A5,
                        flags: ConsumerFlags::none().with_psu_type(PsuType::TypeC),
                    }
                );
                assert!(ptr::eq(psu, port0.port));
            }
            _ => panic!("Did not receive consumer connected event on role swap"),
        }

        // The port should now be tracking a connected consumer internally.
        assert!(matches!(
            port0.port.lock().await.state().psu_state,
            PsuState::ConnectedConsumer(_)
        ));
    }
}

#[tokio::test]
async fn test_basic_consumer_flow() {
    common::run_test(
        DEFAULT_TEST_DURATION,
        Default::default(),
        Default::default(),
        TestBasicConsumerFlow,
    )
    .await;
}

#[tokio::test]
async fn test_basic_provider_flow() {
    common::run_test(
        DEFAULT_TEST_DURATION,
        Default::default(),
        Default::default(),
        TestBasicProviderFlow,
    )
    .await;
}

#[tokio::test]
async fn test_consumer_flow_timer_sink_ready() {
    common::run_test(
        Duration::from_secs(10),
        Default::default(),
        Default::default(),
        TestConsumerFlowTimerSinkReady,
    )
    .await;
}

#[tokio::test]
async fn test_sink_disable_on_voltage_change() {
    common::run_test(
        DEFAULT_TEST_DURATION,
        Default::default(),
        Default::default(),
        TestSinkDisableOnVoltageChange,
    )
    .await;
}

#[tokio::test]
async fn test_set_max_sink_voltage_recovery() {
    common::run_test(
        DEFAULT_TEST_DURATION,
        Default::default(),
        Default::default(),
        TestSetMaxSinkVoltageRecovery,
    )
    .await;
}

#[tokio::test]
async fn test_consumer_to_provider_role_swap() {
    common::run_test(
        DEFAULT_TEST_DURATION,
        Default::default(),
        Default::default(),
        TestConsumerToProviderRoleSwap,
    )
    .await;
}

#[tokio::test]
async fn test_provider_to_consumer_role_swap() {
    common::run_test(
        DEFAULT_TEST_DURATION,
        Default::default(),
        Default::default(),
        TestProviderToConsumerRoleSwap,
    )
    .await;
}
