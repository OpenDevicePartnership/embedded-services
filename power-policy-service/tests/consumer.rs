#![allow(clippy::unwrap_used)]
use embassy_sync::{mutex::Mutex, signal::Signal};
use embassy_time::{Duration, TimeoutError, with_timeout};
use embedded_services::GlobalRawMutex;
use power_policy_service::capability::{ConsumerFlags, ConsumerPowerCapability};

mod common;

use common::LOW_POWER;

use crate::common::{
    DEFAULT_TIMEOUT, HIGH_POWER,
    mock::{FnCall, Mock},
    run_test,
};

const PER_CALL_TIMEOUT: Duration = Duration::from_millis(1000);

/// Test the basic consumer flow with a single device.
async fn test_single(
    device0: &'static Mutex<GlobalRawMutex, Mock<'static>>,
    device0_signal: &'static Signal<GlobalRawMutex, (usize, FnCall)>,
) {
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
    }
}

/// Test swapping to a higher powered device.
async fn test_swap_higher(
    device0: &'static Mutex<GlobalRawMutex, Mock<'static>>,
    device0_signal: &'static Signal<GlobalRawMutex, (usize, FnCall)>,
    device1: &'static Mutex<GlobalRawMutex, Mock<'static>>,
    device1_signal: &'static Signal<GlobalRawMutex, (usize, FnCall)>,
) {
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
    }
}

/// Run all tests, this is temporary to deal with 'static lifetimes until the intrusive list refactor is done.
#[tokio::test]
async fn run_all_tests() {
    run_test(
        DEFAULT_TIMEOUT,
        |device0, device0_signal, device1, device1_signal| async move {
            test_single(device0, device0_signal).await;
            test_swap_higher(device0, device0_signal, device1, device1_signal).await;
        },
    )
    .await;
}
