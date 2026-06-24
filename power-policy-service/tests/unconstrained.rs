#![allow(clippy::unwrap_used)]
use embassy_sync::channel::DynamicReceiver;
use embedded_services::info;
use power_policy_interface::capability::{ConsumerFlags, ConsumerPowerCapability};

mod common;

use common::LOW_POWER;
use power_policy_interface::service::UnconstrainedState;
use power_policy_interface::service::event::Event as ServiceEvent;
use power_policy_service::service::customization::DefaultCustomization;

use crate::common::HIGH_POWER;
use crate::common::{
    DEFAULT_TIMEOUT, assert_consumer_connected, assert_consumer_disconnected, assert_no_event, assert_unconstrained,
    run_test,
};
use crate::common::{DeviceType, ServiceMutex, Test};
use power_policy_interface_test_mocks::psu::FnCall;

/// Test unconstrained consumer flow with multiple devices.
struct TestUnconstrained;

impl Test for TestUnconstrained {
    type Customization = DefaultCustomization;

    async fn run<'a>(
        &mut self,
        _service: &ServiceMutex<'a, 'a, Self::Customization>,
        service_receiver: DynamicReceiver<'a, ServiceEvent<'a, DeviceType<'a>>>,
        device0: &DeviceType<'a>,
        device1: &DeviceType<'a>,
    ) {
        info!("Running test_unconstrained");
        {
            // Connect device0, without unconstrained,
            device0.lock().await.next_result_connect_consumer.push_back(Ok(()));
            device0
                .lock()
                .await
                .simulate_consumer_connection(LOW_POWER.into())
                .await;

            assert_consumer_connected(
                service_receiver,
                device0,
                ConsumerPowerCapability {
                    capability: LOW_POWER,
                    flags: ConsumerFlags::none(),
                },
            )
            .await;

            let mut device = device0.lock().await;
            assert_eq!(
                device.fn_calls.pop_front().unwrap(),
                FnCall::ConnectConsumer(ConsumerPowerCapability {
                    capability: LOW_POWER,
                    flags: ConsumerFlags::none(),
                })
            );
            assert!(device.fn_calls.is_empty());

            // Should not have any unconstrained events
            assert!(service_receiver.try_receive().is_err());
        }

        {
            // Connect device1 with unconstrained at HIGH_POWER to force power policy to select this consumer.
            device0.lock().await.next_result_disconnect.push_back(Ok(()));
            device1.lock().await.next_result_connect_consumer.push_back(Ok(()));
            device1
                .lock()
                .await
                .simulate_consumer_connection(ConsumerPowerCapability {
                    capability: HIGH_POWER,
                    flags: ConsumerFlags::none().with_unconstrained_power(),
                })
                .await;

            // Should receive a disconnect event from device0 first
            assert_consumer_disconnected(service_receiver, device0).await;

            assert_consumer_connected(
                service_receiver,
                device1,
                ConsumerPowerCapability {
                    capability: HIGH_POWER,
                    flags: ConsumerFlags::none().with_unconstrained_power(),
                },
            )
            .await;

            assert_unconstrained(
                service_receiver,
                UnconstrainedState {
                    unconstrained: true,
                    available: 1,
                },
            )
            .await;

            {
                let mut device0 = device0.lock().await;
                assert_eq!(device0.fn_calls.pop_front().unwrap(), FnCall::Disconnect);
                assert!(device0.fn_calls.is_empty());
            }
            {
                let mut device1 = device1.lock().await;
                assert_eq!(
                    device1.fn_calls.pop_front().unwrap(),
                    FnCall::ConnectConsumer(ConsumerPowerCapability {
                        capability: HIGH_POWER,
                        flags: ConsumerFlags::none().with_unconstrained_power(),
                    })
                );
                assert!(device1.fn_calls.is_empty());
            }
        }

        {
            // Test detach device1, unconstrained state should change
            device0.lock().await.next_result_connect_consumer.push_back(Ok(()));
            device1.lock().await.simulate_detach().await;

            // Should receive a disconnect event from device1 first
            assert_consumer_disconnected(service_receiver, device1).await;

            assert_consumer_connected(
                service_receiver,
                device0,
                ConsumerPowerCapability {
                    capability: LOW_POWER,
                    flags: ConsumerFlags::none(),
                },
            )
            .await;

            assert_unconstrained(
                service_receiver,
                UnconstrainedState {
                    unconstrained: false,
                    available: 0,
                },
            )
            .await;

            // Power policy shouldn't call any functions on device1 for detach
            assert!(device1.lock().await.fn_calls.is_empty());

            let mut device0 = device0.lock().await;
            assert_eq!(
                device0.fn_calls.pop_front().unwrap(),
                FnCall::ConnectConsumer(ConsumerPowerCapability {
                    capability: LOW_POWER,
                    flags: ConsumerFlags::none(),
                })
            );
            assert!(device0.fn_calls.is_empty());
        }

        assert_no_event(service_receiver);
    }
}

#[tokio::test]
async fn run_test_unconstrained() {
    run_test(
        DEFAULT_TIMEOUT,
        TestUnconstrained,
        Default::default(),
        DefaultCustomization,
    )
    .await;
}
