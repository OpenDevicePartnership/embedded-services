#![allow(clippy::unwrap_used)]
use embassy_sync::channel::DynamicReceiver;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, TimeoutError, with_timeout};
use embedded_services::GlobalRawMutex;
use embedded_services::info;
use power_policy_interface::capability::{ConsumerFlags, ConsumerPowerCapability};

mod common;

use common::{LOW_POWER, ServiceMutex};
use power_policy_interface::service::event::Event as ServiceEvent;
use power_policy_service::service::config::Config;

use crate::common::DeviceType;
use crate::common::MINIMAL_POWER;
use crate::common::Test;
use crate::common::assert_no_event;
use crate::common::{
    DEFAULT_TIMEOUT, HIGH_POWER, assert_consumer_connected, assert_consumer_disconnected, mock::FnCall, run_test,
};

const PER_CALL_TIMEOUT: Duration = Duration::from_millis(1000);

const MIN_CONSUMER_THRESHOLD_MW: u32 = 7500;

/// Test the basic consumer flow with a single device.
struct TestSingle;

impl Test for TestSingle {
    async fn run<'a>(
        &mut self,
        service: &ServiceMutex<'a, 'a>,
        service_receiver: DynamicReceiver<'a, ServiceEvent<'a, DeviceType<'a>>>,
        device0: &DeviceType<'a>,
        device0_signal: &Signal<GlobalRawMutex, (usize, FnCall)>,
        _device1: &DeviceType<'a>,
        _device1_signal: &Signal<GlobalRawMutex, (usize, FnCall)>,
    ) {
        info!("Running test_single");
        // Test initial connection
        {
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

            assert_consumer_connected(
                service_receiver,
                device0,
                ConsumerPowerCapability {
                    capability: LOW_POWER,
                    flags: ConsumerFlags::none(),
                },
            )
            .await;

            // Ensure consumer change doesn't affect provider power computation
            assert_eq!(service.lock().await.compute_total_provider_power_mw().await, 0);
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

            assert_consumer_disconnected(service_receiver, device0).await;
            // Ensure consumer change doesn't affect provider power computation
            assert_eq!(service.lock().await.compute_total_provider_power_mw().await, 0);
        }

        assert_no_event(service_receiver);
    }
}

/// Test swapping to a higher powered device.
struct TestSwapHigher;

impl Test for TestSwapHigher {
    async fn run<'a>(
        &mut self,
        service: &ServiceMutex<'a, 'a>,
        service_receiver: DynamicReceiver<'a, ServiceEvent<'a, DeviceType<'a>>>,
        device0: &DeviceType<'a>,
        device0_signal: &Signal<GlobalRawMutex, (usize, FnCall)>,
        device1: &DeviceType<'a>,
        device1_signal: &Signal<GlobalRawMutex, (usize, FnCall)>,
    ) {
        info!("Running test_swap_higher");
        // Device0 connection at low power
        {
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

            assert_consumer_connected(
                service_receiver,
                device0,
                ConsumerPowerCapability {
                    capability: LOW_POWER,
                    flags: ConsumerFlags::none(),
                },
            )
            .await;

            // Ensure consumer change doesn't affect provider power computation
            assert_eq!(service.lock().await.compute_total_provider_power_mw().await, 0);
        }
        // Device1 connection at high power
        {
            device1
                .lock()
                .await
                .simulate_consumer_connection(HIGH_POWER.into())
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
                        flags: ConsumerFlags::none(),
                    })
                )
            );
            device1_signal.reset();

            // Should receive a disconnect event from device0 first
            assert_consumer_disconnected(service_receiver, device0).await;

            assert_consumer_connected(
                service_receiver,
                device1,
                ConsumerPowerCapability {
                    capability: HIGH_POWER,
                    flags: ConsumerFlags::none(),
                },
            )
            .await;

            // Ensure consumer change doesn't affect provider power computation
            assert_eq!(service.lock().await.compute_total_provider_power_mw().await, 0);
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

            // Ensure consumer change doesn't affect provider power computation
            assert_eq!(service.lock().await.compute_total_provider_power_mw().await, 0);
        }

        assert_no_event(service_receiver);
    }
}

/// Test a disconnect initiated by the current consumer.
struct TestDisconnect;

impl Test for TestDisconnect {
    async fn run<'a>(
        &mut self,
        service: &ServiceMutex<'a, 'a>,
        service_receiver: DynamicReceiver<'a, ServiceEvent<'a, DeviceType<'a>>>,
        device0: &DeviceType<'a>,
        device0_signal: &Signal<GlobalRawMutex, (usize, FnCall)>,
        device1: &DeviceType<'a>,
        device1_signal: &Signal<GlobalRawMutex, (usize, FnCall)>,
    ) {
        info!("Running test_disconnect");
        // Device0 connection at low power
        {
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

            assert_consumer_connected(
                service_receiver,
                device0,
                ConsumerPowerCapability {
                    capability: LOW_POWER,
                    flags: ConsumerFlags::none(),
                },
            )
            .await;

            // Ensure consumer change doesn't affect provider power computation
            assert_eq!(service.lock().await.compute_total_provider_power_mw().await, 0);
        }
        // Device1 connection at high power
        {
            device1
                .lock()
                .await
                .simulate_consumer_connection(HIGH_POWER.into())
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
                        flags: ConsumerFlags::none(),
                    })
                )
            );
            device1_signal.reset();

            // Should receive a disconnect event from device0 first
            assert_consumer_disconnected(service_receiver, device0).await;

            assert_consumer_connected(
                service_receiver,
                device1,
                ConsumerPowerCapability {
                    capability: HIGH_POWER,
                    flags: ConsumerFlags::none(),
                },
            )
            .await;

            // Ensure consumer change doesn't affect provider power computation
            assert_eq!(service.lock().await.compute_total_provider_power_mw().await, 0);
        }

        // Test disconnect device1, should reconnect device0
        {
            device1.lock().await.simulate_disconnect().await;

            // Power policy shouldn't call any functions on disconnect so we'll timeout
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

            // Consume the disconnect event generated by `simulate_disconnect`
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

            // Ensure consumer change doesn't affect provider power computation
            assert_eq!(service.lock().await.compute_total_provider_power_mw().await, 0);
        }

        assert_no_event(service_receiver);
    }
}

/// Test minimum consumer power logic.
///
/// Config for this test uses [`MIN_CONSUMER_THRESHOLD_MW`].
struct TestMinConsumerPower;

impl Test for TestMinConsumerPower {
    async fn run<'a>(
        &mut self,
        service: &ServiceMutex<'a, 'a>,
        service_receiver: DynamicReceiver<'a, ServiceEvent<'a, DeviceType<'a>>>,
        device0: &DeviceType<'a>,
        device0_signal: &Signal<GlobalRawMutex, (usize, FnCall)>,
        _device1: &DeviceType<'a>,
        _device1_signal: &Signal<GlobalRawMutex, (usize, FnCall)>,
    ) {
        info!("Running test_min_consumer_power");
        // Connect with power below the minimum threshold.
        {
            device0
                .lock()
                .await
                .simulate_consumer_connection(MINIMAL_POWER.into())
                .await;

            // Power policy shouldn't connect, so this call should timeout.
            assert_eq!(
                with_timeout(PER_CALL_TIMEOUT, device0_signal.wait()).await,
                Err(TimeoutError)
            );
            device0_signal.reset();

            // Ensure consumer change doesn't affect provider power computation
            assert_eq!(service.lock().await.compute_total_provider_power_mw().await, 0);
        }

        // Service shouldn't broadcast any events in this case.
        assert_no_event(service_receiver);
    }
}

/// Test that we won't swap if the capabilities are the same
struct TestNoSwap;

impl Test for TestNoSwap {
    async fn run<'a>(
        &mut self,
        service: &ServiceMutex<'a, 'a>,
        service_receiver: DynamicReceiver<'a, ServiceEvent<'a, DeviceType<'a>>>,
        device0: &DeviceType<'a>,
        device0_signal: &Signal<GlobalRawMutex, (usize, FnCall)>,
        device1: &DeviceType<'a>,
        device1_signal: &Signal<GlobalRawMutex, (usize, FnCall)>,
    ) {
        info!("Running test_no_swap");
        // Device0 connection at low power
        {
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

            assert_consumer_connected(
                service_receiver,
                device0,
                ConsumerPowerCapability {
                    capability: LOW_POWER,
                    flags: ConsumerFlags::none(),
                },
            )
            .await;

            // Ensure consumer change doesn't affect provider power computation
            assert_eq!(service.lock().await.compute_total_provider_power_mw().await, 0);
        }
        // Device1 connection at low power, should not cause a swap since capabilities are the same
        {
            device1
                .lock()
                .await
                .simulate_consumer_connection(LOW_POWER.into())
                .await;

            // These should timeout since we shouldn't swap
            assert_eq!(
                with_timeout(PER_CALL_TIMEOUT, device0_signal.wait()).await,
                Err(TimeoutError)
            );
            device0_signal.reset();

            assert_eq!(
                with_timeout(PER_CALL_TIMEOUT, device1_signal.wait()).await,
                Err(TimeoutError)
            );
            device1_signal.reset();

            // Shouldn't affect provider power computation
            assert_eq!(service.lock().await.compute_total_provider_power_mw().await, 0);
        }

        // Service shouldn't broadcast any events in this case since we shouldn't swap.
        assert_no_event(service_receiver);
    }
}
#[tokio::test]
async fn run_test_swap_higher() {
    run_test(DEFAULT_TIMEOUT, TestSwapHigher, Default::default()).await;
}

#[tokio::test]
async fn run_test_single() {
    run_test(DEFAULT_TIMEOUT, TestSingle, Default::default()).await;
}

#[tokio::test]
async fn run_test_disconnect() {
    run_test(DEFAULT_TIMEOUT, TestDisconnect, Default::default()).await;
}

#[tokio::test]
async fn run_test_min_consumer_power() {
    run_test(
        DEFAULT_TIMEOUT,
        TestMinConsumerPower,
        Config {
            min_consumer_threshold_mw: Some(MIN_CONSUMER_THRESHOLD_MW),
            ..Default::default()
        },
    )
    .await;
}

#[tokio::test]
async fn run_test_no_swap() {
    run_test(DEFAULT_TIMEOUT, TestNoSwap, Default::default()).await;
}
