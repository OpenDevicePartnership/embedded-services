#![no_std]
#![no_main]

use embassy_sync::once_lock::OnceLock;
use embedded_mcu_hal::Nvram;
use embedded_services::{error, info};
use static_cell::StaticCell;
use {defmt_rtt as _, panic_probe as _};

mod mock_espi_service {
    use crate::OnceLock;
    use crate::{error, info};
    use embassy_time::{Duration, Ticker};
    use embedded_services::comms::{self, EndpointID, External, Internal};
    use time_alarm_service_messages::{AcpiTimeAlarmRequest, AcpiTimeAlarmResponse};

    pub struct Service {
        endpoint: comms::Endpoint,
    }

    impl Service {
        pub async fn init(spawner: embassy_executor::Spawner, service_storage: &'static OnceLock<Service>) {
            let instance = service_storage.get_or_init(|| Service {
                endpoint: comms::Endpoint::uninit(EndpointID::External(External::Host)),
            });

            comms::register_endpoint(instance, &instance.endpoint).await.unwrap();

            spawner.must_spawn(run_mock_service(instance));
        }
    }

    impl comms::MailboxDelegate for Service {
        fn receive(&self, message: &comms::Message) -> Result<(), comms::MailboxDelegateError> {
            let msg = message.data.get::<AcpiTimeAlarmResponse>().ok_or_else(|| {
                error!("Mock eSPI service received unknown message type");
                comms::MailboxDelegateError::MessageNotFound
            })?;

            info!("Mock eSPI service received ACPI Time Alarm Response: {:?}", msg);

            Ok(())
        }
    }

    #[embassy_executor::task]
    async fn run_mock_service(espi_service: &'static Service) {
        let mut ticker = Ticker::every(Duration::from_secs(1));

        loop {
            ticker.next().await;
            espi_service
                .endpoint
                .send(
                    EndpointID::Internal(Internal::TimeAlarm),
                    &AcpiTimeAlarmRequest::GetRealTime,
                )
                .await
                .unwrap();
        }
    }
}

#[embassy_executor::main]
async fn main(spawner: embassy_executor::Spawner) {
    let p = embassy_imxrt::init(Default::default());

    static RTC: StaticCell<embassy_imxrt::rtc::Rtc> = StaticCell::new();
    let rtc = RTC.init(embassy_imxrt::rtc::Rtc::new(p.RTC));
    let (dt_clock, rtc_nvram) = rtc.split();

    let [tz, ac_expiration, ac_policy, dc_expiration, dc_policy, ..] = rtc_nvram.storage();

    embedded_services::init().await;
    info!("services initialized");

    static MOCK_ESPI_SERVICE: OnceLock<mock_espi_service::Service> = OnceLock::new();
    mock_espi_service::Service::init(spawner, &MOCK_ESPI_SERVICE).await;

    static TIME_SERVICE: embassy_sync::once_lock::OnceLock<time_alarm_service::Service> =
        embassy_sync::once_lock::OnceLock::new();
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
    async fn command_handler_task(service: &'static time_alarm_service::Service) {
        time_alarm_service::task::command_handler_task(service).await
    }

    #[embassy_executor::task]
    async fn ac_timer_task(service: &'static time_alarm_service::Service) {
        time_alarm_service::task::ac_timer_task(service).await
    }

    #[embassy_executor::task]
    async fn dc_timer_task(service: &'static time_alarm_service::Service) {
        time_alarm_service::task::dc_timer_task(service).await
    }

    spawner.must_spawn(command_handler_task(time_service));
    spawner.must_spawn(ac_timer_task(time_service));
    spawner.must_spawn(dc_timer_task(time_service));
}
