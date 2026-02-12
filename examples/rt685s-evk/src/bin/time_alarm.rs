#![no_std]
#![no_main]

use embassy_sync::once_lock::OnceLock;
use embedded_mcu_hal::{
    Nvram,
    time::{Datetime, Month, UncheckedDatetime},
};
use embedded_services::info;
use static_cell::StaticCell;
use time_alarm_service_messages::{AcpiDaylightSavingsTimeStatus, AcpiTimeZone, AcpiTimeZoneOffset, AcpiTimestamp};
use {defmt_rtt as _, panic_probe as _};

#[embassy_executor::main]
async fn main(spawner: embassy_executor::Spawner) {
    let p = embassy_imxrt::init(Default::default());

    static RTC: StaticCell<embassy_imxrt::rtc::Rtc> = StaticCell::new();
    let rtc = RTC.init(embassy_imxrt::rtc::Rtc::new(p.RTC));
    let (dt_clock, rtc_nvram) = rtc.split();

    let [tz, ac_expiration, ac_policy, dc_expiration, dc_policy, ..] = rtc_nvram.storage();

    embedded_services::init().await;
    info!("services initialized");

    static TIME_SERVICE: OnceLock<time_alarm_service::Service> = OnceLock::new();
    let time_service = time_alarm_service::Service::init(
        &TIME_SERVICE,
        dt_clock,
        tz,
        ac_expiration,
        ac_policy,
        dc_expiration,
        dc_policy,
    )
    .await
    .expect("Failed to initialize time-alarm service");

    #[embassy_executor::task]
    async fn time_alarm_task(service: &'static time_alarm_service::Service<'static>) {
        time_alarm_service::task::run_service(service).await
    }

    spawner.must_spawn(time_alarm_task(time_service));

    use embedded_services::relay::mctp::impl_odp_mctp_relay_handler;
    impl_odp_mctp_relay_handler!(
        EspiRelayHandler;
        TimeAlarm, 0x0B, time_alarm_service::Service<'static>;
    );

    let _relay_handler = EspiRelayHandler::new(time_service);

    // Here, you'd normally pass _relay_handler to your relay service (e.g. eSPI service).
    // In this example, we're not leveraging a relay service, so we'll just demonstrate some direct calls.
    //

    time_service
        .set_real_time(AcpiTimestamp {
            datetime: Datetime::new(UncheckedDatetime {
                year: 2024,
                month: Month::January,
                day: 10,
                hour: 12,
                minute: 0,
                second: 0,
                nanosecond: 0,
            })
            .unwrap(),
            time_zone: AcpiTimeZone::MinutesFromUtc(AcpiTimeZoneOffset::new(60 * -8).unwrap()),
            dst_status: AcpiDaylightSavingsTimeStatus::NotAdjusted,
        })
        .unwrap();

    loop {
        embassy_time::Timer::after(embassy_time::Duration::from_secs(10)).await;
        info!("Current time from service: {:?}", time_service.get_real_time().unwrap());
    }
}
