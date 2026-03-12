#![allow(clippy::unwrap_used)]
use embassy_sync::channel::DynamicReceiver;
use embassy_sync::signal::Signal;
use embassy_time::TimeoutError;
use embassy_time::{Duration, with_timeout};
use embedded_services::GlobalRawMutex;
use embedded_services::info;
use power_policy_interface::capability::{ConsumerFlags, ConsumerPowerCapability};

mod common;

use common::LOW_POWER;
use power_policy_interface::service::UnconstrainedState;
use power_policy_interface::service::event::Event as ServiceEvent;

use crate::common::DeviceType;
use crate::common::HIGH_POWER;
use crate::common::{DEFAULT_TIMEOUT, mock::FnCall, run_test};

const PER_CALL_TIMEOUT: Duration = Duration::from_millis(1000);

/// Test unconstrained consumer flow with multiple devices.
async fn test_unconstrained<'a>(
    service_receiver: DynamicReceiver<'a, ServiceEvent<'a, DeviceType<'a>>>,
    device0: &DeviceType<'a>,
    device0_signal: &Signal<GlobalRawMutex, (usize, FnCall)>,
    device1: &DeviceType<'a>,
    device1_signal: &Signal<GlobalRawMutex, (usize, FnCall)>,
) {
    info!("Running test_unconstrained");
    {
        // Connect device0, without unconstrained,
        device0
            .lock()
            .await
            .simulate_consumer_connection(LOW_POWER.into())
            .await;

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

        // Should not have any unconstrained events
        assert!(service_receiver.try_receive().is_err());
    }

    {
        // Connect device1 with unconstrained at HIGH_POWER to force power policy to select this consumer.
        device1
            .lock()
            .await
            .simulate_consumer_connection(ConsumerPowerCapability {
                capability: HIGH_POWER,
                flags: ConsumerFlags::none().with_unconstrained_power(),
            })
            .await;

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
                    flags: ConsumerFlags::none().with_unconstrained_power(),
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
                flags: ConsumerFlags::none().with_unconstrained_power(),
            }
        );

        let ServiceEvent::Unconstrained(unconstrained_state) = service_receiver.receive().await else {
            panic!("Expected Unconstrained event");
        };
        assert_eq!(
            unconstrained_state,
            UnconstrainedState {
                unconstrained: true,
                available: 1,
            }
        );
    }

    {
        // Test detach device1, unconstrained state should change
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

        let ServiceEvent::Unconstrained(unconstrained_state) = service_receiver.receive().await else {
            panic!("Expected Unconstrained event");
        };
        assert_eq!(
            unconstrained_state,
            UnconstrainedState {
                unconstrained: false,
                available: 0,
            }
        );
    }
}

#[tokio::test]
async fn run_test_unconstrained() {
    run_test(DEFAULT_TIMEOUT, test_unconstrained).await;
}
