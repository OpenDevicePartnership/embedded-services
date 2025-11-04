use embassy_time::Timer;
use embedded_services::power::policy::{ConsumerPowerCapability, flags::Consumer};

mod common;

use common::LOW_POWER;

use crate::common::{DEFAULT_TIMEOUT, HIGH_POWER, run_test};

/// Test the basic consumer flow with a single device.
/*#[tokio::test]
async fn test_single() {
    run_test(DEFAULT_TIMEOUT, |device0, _| async {
        // Test initial connection
        {
            device0.lock().await.simulate_consumer_connection(LOW_POWER).await;
            Timer::after_millis(500).await;

            let mut dev0 = device0.lock().await;
            assert_eq!(dev0.num_fn_calls, 1);
            assert_eq!(
                dev0.last_fn_call,
                Some(common::mock::FnCall::ConnectConsumer(ConsumerPowerCapability {
                    capability: LOW_POWER,
                    flags: Consumer::none(),
                }))
            );
            dev0.reset_mock();
        }
        // Test detach
        {
            device0.lock().await.simulate_detach().await;
            Timer::after_millis(500).await;

            let mut dev0 = device0.lock().await;
            // Power policy shouldn't do any function calls
            assert_eq!(dev0.num_fn_calls, 0);
            assert_eq!(dev0.last_fn_call, None,);
            dev0.reset_mock();
        }
    })
    .await;
}*/

/// Test swapping to a higher powered device.
#[tokio::test]
async fn test_swap_higher() {
    run_test(DEFAULT_TIMEOUT, |device0, device1| async {
        // Device0 connection at low power
        {
            device0.lock().await.simulate_consumer_connection(LOW_POWER).await;
            Timer::after_millis(500).await;

            let mut dev0 = device0.lock().await;
            assert_eq!(dev0.num_fn_calls, 1);
            assert_eq!(
                dev0.last_fn_call,
                Some(common::mock::FnCall::ConnectConsumer(ConsumerPowerCapability {
                    capability: LOW_POWER,
                    flags: Consumer::none(),
                }))
            );
            dev0.reset_mock();
        }
        // Device1 connection at high power
        {
            device1.lock().await.simulate_consumer_connection(HIGH_POWER).await;
            Timer::after_millis(500).await;

            // Check that device0 was disconnected
            let mut dev0 = device0.lock().await;
            assert_eq!(dev0.num_fn_calls, 1);
            assert_eq!(dev0.last_fn_call, Some(common::mock::FnCall::Disconnect));
            dev0.reset_mock();

            // Check that device1 was connected
            let mut dev1 = device1.lock().await;
            assert_eq!(dev1.num_fn_calls, 1);
            assert_eq!(
                dev1.last_fn_call,
                Some(common::mock::FnCall::ConnectConsumer(ConsumerPowerCapability {
                    capability: HIGH_POWER,
                    flags: Consumer::none(),
                }))
            );
            dev1.reset_mock();
        }
    })
    .await;
}
