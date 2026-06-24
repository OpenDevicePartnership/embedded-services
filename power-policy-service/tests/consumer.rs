#![allow(clippy::unwrap_used)]
use embassy_sync::channel::DynamicReceiver;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, TimeoutError, with_timeout};
use embedded_services::GlobalRawMutex;
use embedded_services::info;
use embedded_services::sync::Lockable;
use power_policy_interface::capability::ProviderFlags;
use power_policy_interface::capability::ProviderPowerCapability;
use power_policy_interface::capability::{ConsumerDisconnect, ConsumerFlags, ConsumerPowerCapability};

mod common;

use common::{LOW_POWER, ServiceMutex};
use power_policy_interface::psu::Psu;
use power_policy_interface::service::event::Event as ServiceEvent;
use power_policy_service::service::InternalState;
use power_policy_service::service::config::Config;
use power_policy_service::service::consumer::AvailableConsumer;
use power_policy_service::service::consumer::cmp_consumer_capability_default;
use power_policy_service::service::consumer::find_best_consumer_default;
use power_policy_service::service::customization;
use power_policy_service::service::customization::DefaultCustomization;
use power_policy_service::service::registration::Registration;

use crate::common::DeviceType;
use crate::common::MINIMAL_POWER;
use crate::common::Test;
use crate::common::assert_no_event;
use crate::common::assert_provider_connected;
use crate::common::assert_provider_disconnected;
use crate::common::{
    DEFAULT_TIMEOUT, HIGH_POWER, assert_consumer_connected, assert_consumer_disconnected,
    assert_consumer_disconnected_with_flags, run_test,
};
use power_policy_interface_test_mocks::psu::FnCall;

const PER_CALL_TIMEOUT: Duration = Duration::from_millis(1000);

const MIN_CONSUMER_THRESHOLD_MW: u32 = 7500;

/// Test the basic consumer flow with a single device.
struct TestSingle;

impl Test for TestSingle {
    type Customization = DefaultCustomization;

    async fn run<'a>(
        &mut self,
        service: &ServiceMutex<'a, 'a, Self::Customization>,
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
    type Customization = DefaultCustomization;

    async fn run<'a>(
        &mut self,
        service: &ServiceMutex<'a, 'a, Self::Customization>,
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
    type Customization = DefaultCustomization;

    async fn run<'a>(
        &mut self,
        service: &ServiceMutex<'a, 'a, Self::Customization>,
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

/// Test a disconnect initiated by a consumer other than the current consumer.
struct TestDisconnectOtherConsumer;

impl Test for TestDisconnectOtherConsumer {
    type Customization = DefaultCustomization;

    async fn run<'a>(
        &mut self,
        service: &ServiceMutex<'a, 'a, Self::Customization>,
        service_receiver: DynamicReceiver<'a, ServiceEvent<'a, DeviceType<'a>>>,
        device0: &DeviceType<'a>,
        device0_signal: &Signal<GlobalRawMutex, (usize, FnCall)>,
        device1: &DeviceType<'a>,
        device1_signal: &Signal<GlobalRawMutex, (usize, FnCall)>,
    ) {
        info!("Running test_disconnect_other_consumer");
        // Device0 connection at high power
        {
            device0
                .lock()
                .await
                .simulate_consumer_connection(HIGH_POWER.into())
                .await;

            assert_eq!(
                with_timeout(PER_CALL_TIMEOUT, device0_signal.wait()).await.unwrap(),
                (
                    1,
                    FnCall::ConnectConsumer(ConsumerPowerCapability {
                        capability: HIGH_POWER,
                        flags: ConsumerFlags::none(),
                    })
                )
            );
            device0_signal.reset();

            assert_consumer_connected(
                service_receiver,
                device0,
                ConsumerPowerCapability {
                    capability: HIGH_POWER,
                    flags: ConsumerFlags::none(),
                },
            )
            .await;

            // Ensure consumer change doesn't affect provider power computation
            assert_eq!(service.lock().await.compute_total_provider_power_mw().await, 0);
        }
        // Device1 connection at low power
        {
            device1
                .lock()
                .await
                .simulate_consumer_connection(LOW_POWER.into())
                .await;

            assert_eq!(
                with_timeout(PER_CALL_TIMEOUT, device1_signal.wait()).await,
                Err(TimeoutError)
            );
            device1_signal.reset();

            // Ensure consumer change doesn't affect provider power computation
            assert_eq!(service.lock().await.compute_total_provider_power_mw().await, 0);
        }

        // Test disconnect device1, should have no effect on device0
        {
            device1.lock().await.simulate_disconnect().await;

            // Power policy shouldn't call any functions on disconnect so we'll timeout
            assert_eq!(
                with_timeout(PER_CALL_TIMEOUT, device0_signal.wait()).await,
                Err(TimeoutError)
            );
            assert_eq!(
                with_timeout(PER_CALL_TIMEOUT, device1_signal.wait()).await,
                Err(TimeoutError)
            );

            // Ensure consumer change doesn't affect provider power computation
            assert_eq!(service.lock().await.compute_total_provider_power_mw().await, 0);
        }

        assert_no_event(service_receiver);
    }
}

/// Test a disconnect initiated by a provider other than the current consumer.
struct TestDisconnectOtherProvider;

impl Test for TestDisconnectOtherProvider {
    type Customization = DefaultCustomization;

    async fn run<'a>(
        &mut self,
        service: &ServiceMutex<'a, 'a, Self::Customization>,
        service_receiver: DynamicReceiver<'a, ServiceEvent<'a, DeviceType<'a>>>,
        device0: &DeviceType<'a>,
        device0_signal: &Signal<GlobalRawMutex, (usize, FnCall)>,
        device1: &DeviceType<'a>,
        device1_signal: &Signal<GlobalRawMutex, (usize, FnCall)>,
    ) {
        info!("Running test_disconnect_other_provider");
        // Device0 connection at high power
        {
            device0
                .lock()
                .await
                .simulate_consumer_connection(HIGH_POWER.into())
                .await;

            assert_eq!(
                with_timeout(PER_CALL_TIMEOUT, device0_signal.wait()).await.unwrap(),
                (
                    1,
                    FnCall::ConnectConsumer(ConsumerPowerCapability {
                        capability: HIGH_POWER,
                        flags: ConsumerFlags::none(),
                    })
                )
            );
            device0_signal.reset();

            assert_consumer_connected(
                service_receiver,
                device0,
                ConsumerPowerCapability {
                    capability: HIGH_POWER,
                    flags: ConsumerFlags::none(),
                },
            )
            .await;

            // Ensure consumer change doesn't affect provider power computation
            assert_eq!(service.lock().await.compute_total_provider_power_mw().await, 0);
        }
        // Device1 connect as provider
        {
            device1.lock().await.simulate_provider_connection(LOW_POWER).await;

            assert_eq!(
                with_timeout(PER_CALL_TIMEOUT, device1_signal.wait()).await.unwrap(),
                (
                    1,
                    FnCall::ConnectProvider(ProviderPowerCapability {
                        capability: LOW_POWER,
                        flags: ProviderFlags::none(),
                    })
                )
            );
            device1_signal.reset();

            assert_provider_connected(
                service_receiver,
                device1,
                ProviderPowerCapability {
                    capability: LOW_POWER,
                    flags: ProviderFlags::none(),
                },
            )
            .await;
            assert_eq!(service.lock().await.compute_total_provider_power_mw().await, 7500);
        }

        // Test disconnect device1, should have no effect on device0
        {
            device1.lock().await.simulate_disconnect().await;

            // Power policy shouldn't call any functions on disconnect so we'll timeout
            assert_eq!(
                with_timeout(PER_CALL_TIMEOUT, device0_signal.wait()).await,
                Err(TimeoutError)
            );
            assert_eq!(
                with_timeout(PER_CALL_TIMEOUT, device1_signal.wait()).await,
                Err(TimeoutError)
            );

            assert_provider_disconnected(service_receiver, device1).await;
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
    type Customization = DefaultCustomization;

    async fn run<'a>(
        &mut self,
        service: &ServiceMutex<'a, 'a, Self::Customization>,
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
    type Customization = DefaultCustomization;

    async fn run<'a>(
        &mut self,
        service: &ServiceMutex<'a, 'a, Self::Customization>,
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

/// Power policy customization that always returns the first PSU if it's available
struct AlwaysFirstConsumerCustomization;

impl customization::Customization for AlwaysFirstConsumerCustomization {
    async fn find_best_consumer<'device, Reg: Registration<'device>>(
        &mut self,
        config: &Config,
        state: &InternalState<'device, Reg::Psu>,
        registration: &Reg,
    ) -> Result<Option<AvailableConsumer<'device, Reg::Psu>>, power_policy_interface::psu::Error> {
        let psu0 = registration.psus().iter().next().unwrap();
        if let Some(consumer_power_capability) = psu0.lock().await.state().consumer_capability {
            Ok(Some(AvailableConsumer {
                psu: psu0,
                consumer_power_capability,
            }))
        } else {
            find_best_consumer_default(config, state, registration, cmp_consumer_capability_default).await
        }
    }
}

/// Verify that [`customization::Customization::find_best_consumer`] is called
struct TestFindBestConsumerCustomization;

impl Test for TestFindBestConsumerCustomization {
    type Customization = AlwaysFirstConsumerCustomization;

    async fn run<'a>(
        &mut self,
        _service: &ServiceMutex<'a, 'a, Self::Customization>,
        service_receiver: DynamicReceiver<'a, ServiceEvent<'a, DeviceType<'a>>>,
        device0: &DeviceType<'a>,
        device0_signal: &Signal<GlobalRawMutex, (usize, FnCall)>,
        device1: &DeviceType<'a>,
        device1_signal: &Signal<GlobalRawMutex, (usize, FnCall)>,
    ) {
        info!("Running TestFindBestConsumerCustomization");

        // Device1 connection at high power
        {
            device1
                .lock()
                .await
                .simulate_consumer_connection(HIGH_POWER.into())
                .await;

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

            assert_consumer_connected(
                service_receiver,
                device1,
                ConsumerPowerCapability {
                    capability: HIGH_POWER,
                    flags: ConsumerFlags::none(),
                },
            )
            .await;
        }

        // Device0 connection at low power
        // Since we're using a custom hook, we expect to switch to it
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
        }
    }
}

/// Test that disconnecting the current consumer to switch to a different PSU sets the
/// `switching` flag on the [`ServiceEvent::ConsumerDisconnected`] event.
struct TestConsumerDisconnectSwitchingFlag;

impl Test for TestConsumerDisconnectSwitchingFlag {
    type Customization = DefaultCustomization;

    async fn run<'a>(
        &mut self,
        _service: &ServiceMutex<'a, 'a, Self::Customization>,
        service_receiver: DynamicReceiver<'a, ServiceEvent<'a, DeviceType<'a>>>,
        device0: &DeviceType<'a>,
        device0_signal: &Signal<GlobalRawMutex, (usize, FnCall)>,
        device1: &DeviceType<'a>,
        device1_signal: &Signal<GlobalRawMutex, (usize, FnCall)>,
    ) {
        info!("Running test_consumer_disconnect_switching_flag");
        // Connect device0 at low power.
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

        // Connect device1 at high power, the service should switch to it.
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

        // device0 should be disconnected with the switching flag set since we're switching to device1.
        assert_consumer_disconnected_with_flags(
            service_receiver,
            device0,
            ConsumerDisconnect::none().with_switching(true),
        )
        .await;
        assert_consumer_connected(
            service_receiver,
            device1,
            ConsumerPowerCapability {
                capability: HIGH_POWER,
                flags: ConsumerFlags::none(),
            },
        )
        .await;

        assert_no_event(service_receiver);
    }
}

/// Test that disconnecting the current consumer because it renegotiated a new power capability
/// sets the `renegotiation` flag on the [`ServiceEvent::ConsumerDisconnected`] event.
struct TestConsumerDisconnectRenegotiationFlag;

impl Test for TestConsumerDisconnectRenegotiationFlag {
    type Customization = DefaultCustomization;

    async fn run<'a>(
        &mut self,
        _service: &ServiceMutex<'a, 'a, Self::Customization>,
        service_receiver: DynamicReceiver<'a, ServiceEvent<'a, DeviceType<'a>>>,
        device0: &DeviceType<'a>,
        device0_signal: &Signal<GlobalRawMutex, (usize, FnCall)>,
        _device1: &DeviceType<'a>,
        _device1_signal: &Signal<GlobalRawMutex, (usize, FnCall)>,
    ) {
        info!("Running test_consumer_disconnect_renegotiation_flag");
        // Connect device0 at low power.
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

        // The same device renegotiates a new (higher) power capability. Since the best consumer is
        // still the same device but with a different capability, the service disconnects and
        // reconnects it. The disconnect event should carry the renegotiation flag.
        device0
            .lock()
            .await
            .simulate_update_consumer_power_capability(Some(HIGH_POWER.into()))
            .await;

        assert_consumer_disconnected_with_flags(
            service_receiver,
            device0,
            ConsumerDisconnect::none().with_renegotiation(true),
        )
        .await;
        assert_consumer_connected(
            service_receiver,
            device0,
            ConsumerPowerCapability {
                capability: HIGH_POWER,
                flags: ConsumerFlags::none(),
            },
        )
        .await;

        assert_no_event(service_receiver);
    }
}

#[tokio::test]
async fn run_test_swap_higher() {
    run_test(
        DEFAULT_TIMEOUT,
        TestSwapHigher,
        Default::default(),
        DefaultCustomization,
    )
    .await;
}

#[tokio::test]
async fn run_test_single() {
    run_test(DEFAULT_TIMEOUT, TestSingle, Default::default(), DefaultCustomization).await;
}

#[tokio::test]
async fn run_test_disconnect() {
    run_test(
        DEFAULT_TIMEOUT,
        TestDisconnect,
        Default::default(),
        DefaultCustomization,
    )
    .await;
}

#[tokio::test]
async fn run_test_disconnect_other_consumer() {
    run_test(
        DEFAULT_TIMEOUT,
        TestDisconnectOtherConsumer,
        Default::default(),
        DefaultCustomization,
    )
    .await;
}

#[tokio::test]
async fn run_test_disconnect_other_provider() {
    run_test(
        DEFAULT_TIMEOUT,
        TestDisconnectOtherProvider,
        Default::default(),
        DefaultCustomization,
    )
    .await;
}

#[tokio::test]
async fn run_test_min_consumer_power() {
    let mut config = Config::default();
    config.min_consumer_threshold_mw = Some(MIN_CONSUMER_THRESHOLD_MW);

    run_test(DEFAULT_TIMEOUT, TestMinConsumerPower, config, DefaultCustomization).await;
}

#[tokio::test]
async fn run_test_no_swap() {
    run_test(DEFAULT_TIMEOUT, TestNoSwap, Default::default(), DefaultCustomization).await;
}

#[tokio::test]
async fn run_test_find_best_consumer_hook() {
    run_test(
        DEFAULT_TIMEOUT,
        TestFindBestConsumerCustomization,
        Default::default(),
        AlwaysFirstConsumerCustomization,
    )
    .await;
}

#[tokio::test]
async fn run_test_consumer_disconnect_switching_flag() {
    run_test(
        DEFAULT_TIMEOUT,
        TestConsumerDisconnectSwitchingFlag,
        Default::default(),
        DefaultCustomization,
    )
    .await;
}

#[tokio::test]
async fn run_test_consumer_disconnect_renegotiation_flag() {
    run_test(
        DEFAULT_TIMEOUT,
        TestConsumerDisconnectRenegotiationFlag,
        Default::default(),
        DefaultCustomization,
    )
    .await;
}
