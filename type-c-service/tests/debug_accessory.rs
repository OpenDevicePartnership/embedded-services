#![allow(dead_code)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::panic)]

use std::ptr;

use embassy_futures::join::join;
use embassy_time::{TimeoutError, with_timeout};
use embedded_usb_pd::{PowerRole, type_c::ConnectionState};
use power_policy_interface::{
    capability::{ProviderFlags, ProviderPowerCapability, PsuType},
    psu::{Psu, PsuState},
    service::event::Event as PowerPolicyEvent,
};
use type_c_interface::{
    control::pd::PortStatus,
    port::event::{PortEventBitfield, PortStatusEventBitfield},
    service::event::{DebugAccessoryData, EventData as TypeCEventData},
    util::POWER_CAPABILITY_USB_DEFAULT_USB2,
};

use crate::common::{
    DEFAULT_PER_CALL_TIMEOUT, DEFAULT_TEST_DURATION, PortMutexType, PowerPolicyServiceReceiver, Test, TestPort,
    TypeCServiceReceiver,
};

mod common;

/// Receive one type-C service broadcast and assert it is a debug accessory event for `expected_port`.
async fn assert_debug_accessory<'port, 'ch>(
    type_c_receiver: &TypeCServiceReceiver<'port, 'ch>,
    expected_port: &'port PortMutexType<'port, 'ch>,
    connected: bool,
) {
    match with_timeout(DEFAULT_PER_CALL_TIMEOUT, type_c_receiver.receive()).await {
        Ok(event) => {
            assert_eq!(
                event.event,
                TypeCEventData::DebugAccessory(DebugAccessoryData { connected })
            );
            assert!(ptr::eq(event.port, expected_port));
        }
        Err(TimeoutError) => panic!("Did not receive debug accessory event, expected connected: {connected}"),
    }
}

/// Assert that neither the type-C service nor the power policy service broadcast an event.
async fn assert_no_service_broadcast(
    type_c_receiver: &TypeCServiceReceiver<'_, '_>,
    power_policy_receiver: &PowerPolicyServiceReceiver<'_, '_>,
) {
    let (type_c_result, power_policy_result) = join(
        with_timeout(DEFAULT_PER_CALL_TIMEOUT, type_c_receiver.receive()),
        with_timeout(DEFAULT_PER_CALL_TIMEOUT, power_policy_receiver.receive()),
    )
    .await;
    assert_eq!(
        type_c_result.err(),
        Some(TimeoutError),
        "unexpected type-C service broadcast"
    );
    assert_eq!(
        power_policy_result.err(),
        Some(TimeoutError),
        "unexpected power policy broadcast"
    );
}

/// Raise a controller interrupt reporting `status` and drive the port through its event receiver,
/// exercising the same path hardware would take.
async fn simulate_interrupt(port: &mut TestPort<'_, '_>, status: PortStatus, status_event: PortStatusEventBitfield) {
    port.mock.lock().await.next_result_get_port_status.push_back(Ok(status));

    let mut interrupt = PortEventBitfield::none();
    interrupt.status = status_event;
    port.interrupt_sender.send(interrupt).await;

    let event = port.event_receiver.wait_event().await;
    port.port.lock().await.process_event(event).await.unwrap();
}

/// Port status of a debug accessory sourcing USB default current.
const DEBUG_ACCESSORY_SOURCE_STATUS: PortStatus = PortStatus {
    available_source_contract: Some(POWER_CAPABILITY_USB_DEFAULT_USB2),
    connection_state: Some(ConnectionState::DebugAccessory),
    power_role: PowerRole::Source,
    ..PortStatus::new()
};

/// Test the debug accessory connect/disconnect flow end-to-end.
///
/// A controller interrupt reporting a debug accessory that sources USB default current must travel
/// through the port's event receiver and surface as a type-C service
/// [`TypeCEventData::DebugAccessory`] broadcast. Because it also offers a source contract, it must
/// surface as a power policy provider connect/disconnect as well.
struct TestDebugAccessorySource;

impl Test for TestDebugAccessorySource {
    async fn run<'port, 'ch>(
        &mut self,
        type_c_receiver: TypeCServiceReceiver<'port, 'ch>,
        power_policy_receiver: PowerPolicyServiceReceiver<'port, 'ch>,
        mut port0: TestPort<'port, 'ch>,
        _port1: TestPort<'port, 'ch>,
        _port2: TestPort<'port, 'ch>,
    ) {
        // The port should start out detached.
        assert_eq!(port0.port.lock().await.state().psu_state, PsuState::Detached);

        // Simulate a debug accessory connecting as a source.
        let mut status_event = PortStatusEventBitfield::none();
        status_event.set_plug_inserted_or_removed(true);
        status_event.set_new_power_contract_as_provider(true);
        simulate_interrupt(&mut port0, DEBUG_ACCESSORY_SOURCE_STATUS, status_event).await;

        let (_, power_policy_result) = join(
            assert_debug_accessory(&type_c_receiver, port0.port, true),
            with_timeout(DEFAULT_PER_CALL_TIMEOUT, power_policy_receiver.receive()),
        )
        .await;

        // The source contract should also reach the power policy service.
        match power_policy_result {
            Ok(PowerPolicyEvent::ProviderConnected(psu, capability)) => {
                assert_eq!(
                    capability,
                    ProviderPowerCapability {
                        capability: POWER_CAPABILITY_USB_DEFAULT_USB2,
                        flags: ProviderFlags {
                            psu_type: Some(PsuType::TypeC),
                        },
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

        // Simulate the debug accessory disconnecting.
        let mut status_event = PortStatusEventBitfield::none();
        status_event.set_plug_inserted_or_removed(true);
        simulate_interrupt(&mut port0, PortStatus::new(), status_event).await;

        let (_, power_policy_result) = join(
            assert_debug_accessory(&type_c_receiver, port0.port, false),
            with_timeout(DEFAULT_PER_CALL_TIMEOUT, power_policy_receiver.receive()),
        )
        .await;

        match power_policy_result {
            Ok(PowerPolicyEvent::ProviderDisconnected(psu)) => {
                assert!(ptr::eq(psu, port0.port));
            }
            _ => panic!("Did not receive provider disconnected event"),
        }

        assert_eq!(port0.port.lock().await.state().psu_state, PsuState::Detached);
    }
}

/// Test that a regular connection never reports a debug accessory.
struct TestNonDebugAttach;

impl Test for TestNonDebugAttach {
    async fn run<'port, 'ch>(
        &mut self,
        type_c_receiver: TypeCServiceReceiver<'port, 'ch>,
        power_policy_receiver: PowerPolicyServiceReceiver<'port, 'ch>,
        mut port0: TestPort<'port, 'ch>,
        _port1: TestPort<'port, 'ch>,
        _port2: TestPort<'port, 'ch>,
    ) {
        // Simulate a plain connection with no power contract.
        let mut status_event = PortStatusEventBitfield::none();
        status_event.set_plug_inserted_or_removed(true);
        simulate_interrupt(
            &mut port0,
            PortStatus {
                connection_state: Some(ConnectionState::Attached),
                ..Default::default()
            },
            status_event,
        )
        .await;

        assert_no_service_broadcast(&type_c_receiver, &power_policy_receiver).await;

        // Simulate the disconnection.
        let mut status_event = PortStatusEventBitfield::none();
        status_event.set_plug_inserted_or_removed(true);
        simulate_interrupt(&mut port0, PortStatus::new(), status_event).await;

        assert_no_service_broadcast(&type_c_receiver, &power_policy_receiver).await;
    }
}

/// Test that a connected debug accessory is only reported once.
///
/// Covers the connection-changed case. Further status events that leave the connection state
/// untouched must not re-broadcast the debug accessory.
struct TestDebugAccessoryNoRenotify;

impl Test for TestDebugAccessoryNoRenotify {
    async fn run<'port, 'ch>(
        &mut self,
        type_c_receiver: TypeCServiceReceiver<'port, 'ch>,
        power_policy_receiver: PowerPolicyServiceReceiver<'port, 'ch>,
        mut port0: TestPort<'port, 'ch>,
        _port1: TestPort<'port, 'ch>,
        _port2: TestPort<'port, 'ch>,
    ) {
        // Connect a debug accessory without a power contract to keep the power policy service quiet.
        let status = PortStatus {
            connection_state: Some(ConnectionState::DebugAccessory),
            ..Default::default()
        };

        let mut status_event = PortStatusEventBitfield::none();
        status_event.set_plug_inserted_or_removed(true);
        simulate_interrupt(&mut port0, status, status_event).await;

        assert_debug_accessory(&type_c_receiver, port0.port, true).await;

        // A status event that doesn't change the connection state must not re-notify.
        let mut status_event = PortStatusEventBitfield::none();
        status_event.set_alt_mode_entered(true);
        simulate_interrupt(&mut port0, status, status_event).await;

        assert_no_service_broadcast(&type_c_receiver, &power_policy_receiver).await;
    }
}

#[tokio::test]
async fn test_debug_accessory_source() {
    common::run_test(
        DEFAULT_TEST_DURATION,
        Default::default(),
        Default::default(),
        TestDebugAccessorySource,
    )
    .await;
}

#[tokio::test]
async fn test_non_debug_attach() {
    common::run_test(
        DEFAULT_TEST_DURATION,
        Default::default(),
        Default::default(),
        TestNonDebugAttach,
    )
    .await;
}

#[tokio::test]
async fn test_debug_accessory_no_renotify() {
    common::run_test(
        DEFAULT_TEST_DURATION,
        Default::default(),
        Default::default(),
        TestDebugAccessoryNoRenotify,
    )
    .await;
}
