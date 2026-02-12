mod common;

#[cfg(test)]
mod test {
    use embassy_sync::once_lock::OnceLock;
    use embassy_time::Timer;
    use embedded_mcu_hal::time::{Datetime, DatetimeClock};

    use time_alarm_service_messages as msg;

    use crate::common::mocks::*;

    #[tokio::test]
    async fn test_get_time() {
        let mut tz_storage = MockNvramStorage::new(0);
        let mut ac_exp_storage = MockNvramStorage::new(0);
        let mut ac_pol_storage = MockNvramStorage::new(0);
        let mut dc_exp_storage = MockNvramStorage::new(0);
        let mut dc_pol_storage = MockNvramStorage::new(0);

        let mut clock = MockDatetimeClock::new_running();

        let storage = OnceLock::new();
        let service = time_alarm_service::Service::init(
            &storage,
            &mut clock,
            &mut tz_storage,
            &mut ac_exp_storage,
            &mut ac_pol_storage,
            &mut dc_exp_storage,
            &mut dc_pol_storage,
        )
        .await
        .unwrap();

        // We need to have the service have non-static lifetime for our test use cases so we can have
        // multiple test cases.  This means we can't spawn tasks that require 'static lifetime.
        //
        // Instead, we'll use select! to run the worker task in the local scope, which lets us take
        // borrows from the stack and not require 'static.  The worker task is expected to
        // return !, so we should go until the test arm completes and then shut down.
        //
        tokio::select! {
            _ = time_alarm_service::task::run_service(service) => unreachable!("time alarm service task finished unexpectedly"),
            _ = async {
                let delay_secs = 2;
                let begin = service.get_real_time().unwrap();
                println!("Current time from service: {:?}", begin);
                Timer::after(embassy_time::Duration::from_millis(delay_secs * 1000)).await;
                let end = service.get_real_time().unwrap();
                println!("Current time from service after delay: {:?}", end);
                assert!(end.datetime.to_unix_time_seconds() - begin.datetime.to_unix_time_seconds() <= delay_secs + 1);
                assert!(end.datetime.to_unix_time_seconds() - begin.datetime.to_unix_time_seconds() >= delay_secs - 1);
            } => {}
        }
    }

    #[tokio::test]
    async fn test_set_time() {
        let mut tz_storage = MockNvramStorage::new(0);
        let mut ac_exp_storage = MockNvramStorage::new(0);
        let mut ac_pol_storage = MockNvramStorage::new(0);
        let mut dc_exp_storage = MockNvramStorage::new(0);
        let mut dc_pol_storage = MockNvramStorage::new(0);

        let mut clock = MockDatetimeClock::new_paused();
        const TEST_UNIX_TIME: u64 = 1_234_567_890;
        clock
            .set_current_datetime(&Datetime::from_unix_time_seconds(TEST_UNIX_TIME))
            .unwrap();

        let storage = OnceLock::new();
        let service = time_alarm_service::Service::init(
            &storage,
            &mut clock,
            &mut tz_storage,
            &mut ac_exp_storage,
            &mut ac_pol_storage,
            &mut dc_exp_storage,
            &mut dc_pol_storage,
        )
        .await
        .unwrap();

        tokio::select! {
            _ = time_alarm_service::task::run_service(service) => unreachable!("time alarm service task finished unexpectedly"),
            _ = async {
                // Clock is paused, so time shouldn't advance unless we set it.
                let begin = service.get_real_time().unwrap();
                assert_eq!(begin.datetime.to_unix_time_seconds(), TEST_UNIX_TIME);

                let target_timestamp = msg::AcpiTimestamp {
                    datetime: Datetime::from_unix_time_seconds(TEST_UNIX_TIME),
                    time_zone: msg::AcpiTimeZone::Unknown,
                    dst_status: msg::AcpiDaylightSavingsTimeStatus::Adjusted,
                };
                service.set_real_time(target_timestamp.clone()).unwrap();

                let actual_timestamp = service.get_real_time().unwrap();
                assert_eq!(actual_timestamp, target_timestamp);

            } => {}
        }
    }
}
