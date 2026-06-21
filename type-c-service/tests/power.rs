#![allow(clippy::unwrap_used)]
#![allow(clippy::panic)]
use std::ptr;

use embassy_futures::join::join;
use embassy_time::{TimeoutError, with_timeout};
use embedded_usb_pd::{PowerRole, type_c::ConnectionState};
use power_policy_interface::{
    capability::{ConsumerFlags, ConsumerPowerCapability, PsuType},
    service::event::Event as PowerPolicyEvent,
};
use type_c_interface::{
    control::pd::PortStatus,
    port::event::{PortEvent, PortStatusEventBitfield},
    util::POWER_CAPABILITY_5V_1A5,
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
            Ok(PowerPolicyEvent::ConsumerDisconnected(psu)) => {
                assert!(ptr::eq(psu, port0.port));
            }
            _ => panic!("Did not receive consumer disconnected event"),
        }
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
