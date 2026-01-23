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
    use core::borrow::{Borrow, BorrowMut};
    use embassy_time::{Duration, Ticker};
    use embedded_services::buffer::OwnedRef;
    use embedded_services::comms::{self, EndpointID, External, Internal};
    use embedded_services::ec_type::message::AcpiMsgComms; // TODO this is gone, rewrite
    use embedded_services::ec_type::message::HostMsg;

    embedded_services::define_static_buffer!(acpi_buf, u8, [0u8; 69]);

    pub struct Service {
        endpoint: comms::Endpoint,
        acpi_buf_owned_ref: OwnedRef<'static, u8>,
    }

    impl Service {
        pub async fn init(spawner: embassy_executor::Spawner, service_storage: &'static OnceLock<Service>) {
            let instance = service_storage.get_or_init(|| Service {
                endpoint: comms::Endpoint::uninit(EndpointID::External(External::Host)),
                acpi_buf_owned_ref: acpi_buf::get_mut().unwrap(),
            });

            comms::register_endpoint(instance, &instance.endpoint).await.unwrap();

            spawner.must_spawn(run_mock_service(instance));
        }
    }

    impl comms::MailboxDelegate for Service {
        fn receive(&self, message: &comms::Message) -> Result<(), comms::MailboxDelegateError> {
            info!("mock eSPI service received message from time-alarm service");
            let msg = message.data.get::<HostMsg>().ok_or_else(|| {
                error!("Mock eSPI service received unknown message type");
                comms::MailboxDelegateError::MessageNotFound
            })?;

            match msg {
                HostMsg::Notification(n) => {
                    info!("Notification: offset={}", n.offset);
                }
                HostMsg::Response(acpi) => {
                    let payload = acpi.payload.borrow();
                    let payload_slice: &[u8] = payload.borrow();
                    info!(
                        "Response: payload_len={}, payload={:?}",
                        acpi.payload_len,
                        &payload_slice[..acpi.payload_len]
                    );
                }
            }

            Ok(())
        }
    }

    // espi service that will update the memory map
    #[embassy_executor::task]
    async fn run_mock_service(espi_service: &'static Service) {
        let mut ticker = Ticker::every(Duration::from_secs(1));

        loop {
            // let event = select(espi_service.signal.wait(), ticker.next()).await;
            ticker.next().await;

            let payload_len = {
                // TODO alternate between different messages.
                let mut buffer_access = espi_service.acpi_buf_owned_ref.borrow_mut();
                let buffer: &mut [u8] = buffer_access.borrow_mut();
                buffer[0] = 2;

                4 // u32
            };

            let message = AcpiMsgComms {
                payload: acpi_buf::get(),
                payload_len,
            };

            espi_service
                .endpoint
                .send(EndpointID::Internal(Internal::TimeAlarm), &message)
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

    static TIME_ALARM_SERVICE: OnceLock<time_alarm_service::Service> = OnceLock::new();
    time_alarm_service::Service::init(
        &TIME_ALARM_SERVICE,
        &spawner,
        dt_clock,
        tz,
        ac_expiration,
        ac_policy,
        dc_expiration,
        dc_policy,
    )
    .await
    .unwrap();
}
