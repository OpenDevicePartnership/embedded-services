#![no_std]
#![no_main]

use embedded_mcu_hal::{
    Nvram,
    time::{Datetime, Month, UncheckedDatetime},
};
use embedded_services::broadcaster::single_publisher::SinglePublisherChannel;
use embedded_services::info;
use static_cell::StaticCell;
use time_alarm_service_messages::{
    AcpiDaylightSavingsTimeStatus, AcpiTimeZone, AcpiTimeZoneOffset, AcpiTimestamp, TimeAlarmMessage,
};
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

    // All services will basically need to create a SinglePublisherChannel for themsevles.
    // The idea is they consume the sole DynPublisher while anyone who wants to listen to the service
    // will then consume a DynSubscriber from this channel.
    static TIME_ALARM_CHANNEL: SinglePublisherChannel<TimeAlarmMessage, 1, 1> = SinglePublisherChannel::new();
    let time_alarm_publisher = TIME_ALARM_CHANNEL.publisher().unwrap();
    let time_alarm_subscriber = TIME_ALARM_CHANNEL.subscriber().unwrap();

    let time_service = odp_service_common::spawn_service!(
        spawner,
        time_alarm_service::Service<'static>,
        time_alarm_service::InitParams {
            backing_clock: dt_clock,
            tz_storage: tz,
            ac_expiration_storage: ac_expiration,
            ac_policy_storage: ac_policy,
            dc_expiration_storage: dc_expiration,
            dc_policy_storage: dc_policy,
            message_publisher: time_alarm_publisher
        }
    )
    .expect("Failed to spawn time alarm service");

    // Now in the macro we need to associate a notification ID with each service specified here.
    // This is distinct from service_id (which, in this example, is 0x0B for the time alarm service),
    // since the service_id is used for routing purposes whereas the notification_id is used
    // to identify, for example, which IRQ offset to use in the espi service
    use embedded_services::relay::mctp::impl_odp_mctp_relay_handler;
    impl_odp_mctp_relay_handler!(
        EspiRelayHandler;
        TimeAlarm, 0x0B, 1 /* Notification id example */, time_alarm_service::Service<'static>;
    );

    // Here we pass a subscriber into the relay handler so it can listen for notifications from the time alarm service
    // and then relay those to the host SoC over eSPI when they occur
    let _relay_handler = EspiRelayHandler::new(&time_service, time_alarm_subscriber);

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
