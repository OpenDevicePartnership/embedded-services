//! Basic test of type-C service functionality

use embassy_sync::{
    mutex::Mutex,
    pubsub::{DynSubscriber, WaitResult},
};
use embassy_time::with_timeout;
use embedded_services::{
    GlobalRawMutex, info, power,
    type_c::{self, comms::DebugAccessoryMessage},
};
use embedded_usb_pd::type_c::Current;

use crate::common::{DEFAULT_PER_CALL_TIMEOUT, DEFAULT_TEST_DURATION, PORT0_ID, Test, mock};

mod common;

struct TestService;

impl Test for TestService {
    async fn run(
        &mut self,
        mut type_c_receiver: DynSubscriber<'static, type_c::comms::CommsMessage>,
        _power_policy_event_receiver: DynSubscriber<'static, power::policy::CommsMessage>,
        port0: &'static Mutex<GlobalRawMutex, mock::ControllerState<'static>>,
        _port1: &'static Mutex<GlobalRawMutex, mock::ControllerState<'static>>,
        _port2: &'static Mutex<GlobalRawMutex, mock::ControllerState<'static>>,
    ) {
        info!("Simulating debug accessory connection");
        port0
            .lock()
            .await
            .connect_debug_accessory_source(Current::UsbDefault)
            .await;
        let message = with_timeout(DEFAULT_PER_CALL_TIMEOUT, type_c_receiver.next_message()).await;
        assert_eq!(
            message,
            Ok(WaitResult::Message(type_c::comms::CommsMessage::DebugAccessory(
                DebugAccessoryMessage {
                    port: PORT0_ID,
                    connected: true
                }
            )))
        );

        info!("Simulating debug accessory disconnection");
        port0.lock().await.disconnect().await;
        let message = with_timeout(DEFAULT_PER_CALL_TIMEOUT, type_c_receiver.next_message()).await;
        assert_eq!(
            message,
            Ok(WaitResult::Message(type_c::comms::CommsMessage::DebugAccessory(
                DebugAccessoryMessage {
                    port: PORT0_ID,
                    connected: false
                }
            )))
        );
    }
}

#[tokio::test]
async fn service() {
    common::run_test(
        DEFAULT_TEST_DURATION,
        type_c_service::service::config::Config::default(),
        power_policy_service::config::Config::default(),
        TestService,
    )
    .await;
}
