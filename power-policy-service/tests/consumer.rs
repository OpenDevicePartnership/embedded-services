#![allow(clippy::unwrap_used)]
use embassy_sync::channel::DynamicReceiver;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, TimeoutError, with_timeout};
use embedded_services::GlobalRawMutex;
use embedded_services::info;
use power_policy_interface::capability::{ConsumerFlags, ConsumerPowerCapability};

mod common;

use common::LOW_POWER;
use power_policy_interface::service::event::Event as ServiceEvent;

use crate::common::DeviceType;
use crate::common::{DEFAULT_TIMEOUT, HIGH_POWER, mock::FnCall, run_test};

const PER_CALL_TIMEOUT: Duration = Duration::from_millis(1000);

/// Test the basic consumer flow with a single device.
async fn test_single<'a>(
    service_receiver: DynamicReceiver<'a, ServiceEvent<'a, DeviceType<'a>>>,
    device0: &DeviceType<'a>,
    device0_signal: &Signal<GlobalRawMutex, (usize, FnCall)>,
    _device1: &DeviceType<'a>,
    _device1_signal: &Signal<GlobalRawMutex, (usize, FnCall)>,
) {
    info!("Running test_single");
    // Test initial connection
    {
        device0.lock().await.simulate_consumer_connection(LOW_POWER).await;

        assert_eq!(
            with_timeout(PER_CALL_TIMEOUT, device0_signal.wait()).await.unwrap(),
            (
                1,
                FnCall::ConnectConsumer(ConsumerPowerCapability {
                    capability: LOW_POWER,
                    flags: ConsumerFlags::none(),
                })
            )
        );
        device0_signal.reset();

        let ServiceEvent::ConsumerConnected(device, capability) = service_receiver.receive().await else {
            panic!("Expected ConsumerConnected event");
        };
        assert_eq!(device as *const _, device0 as *const _);
        assert_eq!(
            capability,
            ConsumerPowerCapability {
                capability: LOW_POWER,
                flags: ConsumerFlags::none(),
            }
        );
    }
    // Test detach
    {
        device0.lock().await.simulate_detach().await;

        // Power policy shouldn't call any functions on detach so we'll timeout
        assert_eq!(
            with_timeout(PER_CALL_TIMEOUT, device0_signal.wait()).await,
            Err(TimeoutError)
        );
        device0_signal.reset();

        let ServiceEvent::ConsumerDisconnected(device) = service_receiver.receive().await else {
            panic!("Expected ConsumerDisconnect event");
        };
        assert_eq!(device as *const _, device0 as *const _);
    }
}

/// Test swapping to a higher powered device.
async fn test_swap_higher<'a>(
    service_receiver: DynamicReceiver<'a, ServiceEvent<'a, DeviceType<'a>>>,
    device0: &DeviceType<'a>,
    device0_signal: &Signal<GlobalRawMutex, (usize, FnCall)>,
    device1: &DeviceType<'a>,
    device1_signal: &Signal<GlobalRawMutex, (usize, FnCall)>,
) {
    info!("Running test_swap_higher");
    // Device0 connection at low power
    {
        device0.lock().await.simulate_consumer_connection(LOW_POWER).await;

        assert_eq!(
            with_timeout(PER_CALL_TIMEOUT, device0_signal.wait()).await.unwrap(),
            (
                1,
                FnCall::ConnectConsumer(ConsumerPowerCapability {
                    capability: LOW_POWER,
                    flags: ConsumerFlags::none(),
                })
            )
        );
        device0_signal.reset();

        let ServiceEvent::ConsumerConnected(device, capability) = service_receiver.receive().await else {
            panic!("Expected ConsumerConnected event");
        };
        assert_eq!(device as *const _, device0 as *const _);
        assert_eq!(
            capability,
            ConsumerPowerCapability {
                capability: LOW_POWER,
                flags: ConsumerFlags::none(),
            }
        );
    }
    // Device1 connection at high power
    {
        device1.lock().await.simulate_consumer_connection(HIGH_POWER).await;

        assert_eq!(
            with_timeout(PER_CALL_TIMEOUT, device0_signal.wait()).await.unwrap(),
            (1, FnCall::Disconnect)
        );
        device0_signal.reset();

        assert_eq!(
            with_timeout(PER_CALL_TIMEOUT, device1_signal.wait()).await.unwrap(),
            (
                1,
                FnCall::ConnectConsumer(ConsumerPowerCapability {
                    capability: HIGH_POWER,
                    flags: ConsumerFlags::none(),
                })
            )
        );
        device1_signal.reset();

        // Should receive a disconnect event from device0 first
        let ServiceEvent::ConsumerDisconnected(device) = service_receiver.receive().await else {
            panic!("Expected ConsumerDisconnect event");
        };
        assert_eq!(device as *const _, device0 as *const _);

        let ServiceEvent::ConsumerConnected(device, capability) = service_receiver.receive().await else {
            panic!("Expected ConsumerConnected event");
        };
        assert_eq!(device as *const _, device1 as *const _);
        assert_eq!(
            capability,
            ConsumerPowerCapability {
                capability: HIGH_POWER,
                flags: ConsumerFlags::none(),
            }
        );
    }
    // Test detach device1, should reconnect device0
    {
        device1.lock().await.simulate_detach().await;

        // Power policy shouldn't call any functions on detach so we'll timeout
        assert_eq!(
            with_timeout(PER_CALL_TIMEOUT, device1_signal.wait()).await,
            Err(TimeoutError)
        );

        assert_eq!(
            with_timeout(PER_CALL_TIMEOUT, device0_signal.wait()).await.unwrap(),
            (
                1,
                FnCall::ConnectConsumer(ConsumerPowerCapability {
                    capability: LOW_POWER,
                    flags: ConsumerFlags::none(),
                })
            )
        );
        device0_signal.reset();

        // Should receive a disconnect event from device0 first
        let ServiceEvent::ConsumerDisconnected(device) = service_receiver.receive().await else {
            panic!("Expected ConsumerDisconnect event");
        };
        assert_eq!(device as *const _, device1 as *const _);

        let ServiceEvent::ConsumerConnected(device, capability) = service_receiver.receive().await else {
            panic!("Expected ConsumerConnected event");
        };
        assert_eq!(device as *const _, device0 as *const _);
        assert_eq!(
            capability,
            ConsumerPowerCapability {
                capability: LOW_POWER,
                flags: ConsumerFlags::none(),
            }
        );
    }
}

#[tokio::test]
async fn run_test_swap_higher() {
    run_test(DEFAULT_TIMEOUT, test_swap_higher).await;
}

#[tokio::test]
async fn run_test_single() {
    run_test(DEFAULT_TIMEOUT, test_single).await;
}
