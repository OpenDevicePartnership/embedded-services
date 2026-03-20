#![allow(clippy::unwrap_used)]
use embassy_sync::channel::DynamicReceiver;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, TimeoutError, with_timeout};
use embedded_services::GlobalRawMutex;
use embedded_services::info;
use power_policy_interface::capability::ProviderFlags;
use power_policy_interface::capability::ProviderPowerCapability;

mod common;

use common::LOW_POWER;
use power_policy_interface::service::event::Event as ServiceEvent;

use crate::common::DeviceType;
use crate::common::HIGH_POWER;
use crate::common::assert_no_event;
use crate::common::{DEFAULT_TIMEOUT, assert_provider_connected, assert_provider_disconnected, mock::FnCall, run_test};

const PER_CALL_TIMEOUT: Duration = Duration::from_millis(1000);

/// Test the basic provider flow with a single device.
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
        device0.lock().await.simulate_provider_connection(LOW_POWER).await;

        assert_eq!(
            with_timeout(PER_CALL_TIMEOUT, device0_signal.wait()).await.unwrap(),
            (
                1,
                FnCall::ConnectProvider(ProviderPowerCapability {
                    capability: LOW_POWER,
                    flags: ProviderFlags::none(),
                })
            )
        );
        device0_signal.reset();

        assert_provider_connected(
            service_receiver,
            device0,
            ProviderPowerCapability {
                capability: LOW_POWER,
                flags: ProviderFlags::none(),
            },
        )
        .await;
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

        assert_provider_disconnected(service_receiver, device0).await;
    }

    assert_no_event(service_receiver);
}

/// Test provider flow involving multiple devices and upgrading a provider's power capability.
async fn test_upgrade<'a>(
    service_receiver: DynamicReceiver<'a, ServiceEvent<'a, DeviceType<'a>>>,
    device0: &DeviceType<'a>,
    device0_signal: &Signal<GlobalRawMutex, (usize, FnCall)>,
    device1: &DeviceType<'a>,
    device1_signal: &Signal<GlobalRawMutex, (usize, FnCall)>,
) {
    info!("Running test_upgrade");
    {
        // Connect device0 at high power, default service config should allow this
        device0.lock().await.simulate_provider_connection(HIGH_POWER).await;

        assert_eq!(
            with_timeout(PER_CALL_TIMEOUT, device0_signal.wait()).await.unwrap(),
            (
                1,
                FnCall::ConnectProvider(ProviderPowerCapability {
                    capability: HIGH_POWER,
                    flags: ProviderFlags::none(),
                })
            )
        );
        device0_signal.reset();

        assert_provider_connected(
            service_receiver,
            device0,
            ProviderPowerCapability {
                capability: HIGH_POWER,
                flags: ProviderFlags::none(),
            },
        )
        .await;
    }

    {
        // Connect device1 at low power, default service config should allow this
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
    }

    {
        // Attempt to upgrade device1 to high power, power policy should reject this since device0 is already connected at high power
        // Power policy will instead allow us to connect at low power
        device1
            .lock()
            .await
            .simulate_update_requested_provider_power_capability(Some(HIGH_POWER.into()))
            .await;

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
    }

    {
        // Detach device0, this should allow us to upgrade device1 to high power
        device0.lock().await.simulate_detach().await;

        // Power policy shouldn't call any functions on detach so we'll timeout
        assert_eq!(
            with_timeout(PER_CALL_TIMEOUT, device0_signal.wait()).await,
            Err(TimeoutError)
        );
        device0_signal.reset();

        assert_provider_disconnected(service_receiver, device0).await;
    }

    {
        // Attempt to upgrade device1 to high power should now succeed
        device1
            .lock()
            .await
            .simulate_update_requested_provider_power_capability(Some(HIGH_POWER.into()))
            .await;

        assert_eq!(
            with_timeout(PER_CALL_TIMEOUT, device1_signal.wait()).await.unwrap(),
            (
                1,
                FnCall::ConnectProvider(ProviderPowerCapability {
                    capability: HIGH_POWER,
                    flags: ProviderFlags::none(),
                })
            )
        );
        device1_signal.reset();

        assert_provider_connected(
            service_receiver,
            device1,
            ProviderPowerCapability {
                capability: HIGH_POWER,
                flags: ProviderFlags::none(),
            },
        )
        .await;
    }

    assert_no_event(service_receiver);
}

/// Test the provider disconnect flow
async fn test_disconnect<'a>(
    service_receiver: DynamicReceiver<'a, ServiceEvent<'a, DeviceType<'a>>>,
    device0: &DeviceType<'a>,
    device0_signal: &Signal<GlobalRawMutex, (usize, FnCall)>,
    _device1: &DeviceType<'a>,
    _device1_signal: &Signal<GlobalRawMutex, (usize, FnCall)>,
) {
    info!("Running test_disconnect");
    // Test initial connection
    {
        device0.lock().await.simulate_provider_connection(LOW_POWER).await;

        assert_eq!(
            with_timeout(PER_CALL_TIMEOUT, device0_signal.wait()).await.unwrap(),
            (
                1,
                FnCall::ConnectProvider(ProviderPowerCapability {
                    capability: LOW_POWER,
                    flags: ProviderFlags::none(),
                })
            )
        );
        device0_signal.reset();

        assert_provider_connected(
            service_receiver,
            device0,
            ProviderPowerCapability {
                capability: LOW_POWER,
                flags: ProviderFlags::none(),
            },
        )
        .await;
    }
    // Test disconnect
    {
        device0.lock().await.simulate_disconnect().await;

        // Power policy shouldn't call any functions on disconnect so we'll timeout
        assert_eq!(
            with_timeout(PER_CALL_TIMEOUT, device0_signal.wait()).await,
            Err(TimeoutError)
        );
        device0_signal.reset();

        assert_provider_disconnected(service_receiver, device0).await;
    }

    assert_no_event(service_receiver);
}

#[tokio::test]
async fn run_test_single() {
    run_test(DEFAULT_TIMEOUT, test_single).await;
}

#[tokio::test]
async fn run_test_upgrade() {
    run_test(DEFAULT_TIMEOUT, test_upgrade).await;
}

#[tokio::test]
async fn run_test_disconnect() {
    run_test(DEFAULT_TIMEOUT, test_disconnect).await;
}
