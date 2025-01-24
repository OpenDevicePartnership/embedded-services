#![no_std]
#![no_main]

extern crate embedded_services_examples;

use ::espi_service::Message;
use defmt::info;
use embassy_executor::Spawner;
use embedded_services::comms;
use espi_service::espi_service;

// Mock battery service
mod battery_service {
    use defmt::info;
    use embassy_sync::blocking_mutex::raw::NoopRawMutex;
    use embassy_sync::once_lock::OnceLock;
    use embassy_sync::signal::Signal;
    use embedded_services::comms::{self, EndpointID, External, Internal};
    use espi_service::Message;

    struct Service {
        endpoint: comms::Endpoint,

        // This is can be an Embassy signal or channel or whatever Embassy async notification construct
        signal: Signal<NoopRawMutex, espi_service::Message>,
    }

    impl Service {
        fn new() -> Self {
            Service {
                endpoint: comms::Endpoint::uninit(EndpointID::Internal(Internal::Battery)),
                signal: Signal::new(),
            }
        }
    }

    impl comms::MailboxDelegate for Service {
        fn receive(&self, message: &comms::Message) {
            if let Some(msg) = message.data.get::<espi_service::Message>() {
                self.signal.signal(*msg);
            }
        }
    }

    static BATTERY_SERVICE: OnceLock<Service> = OnceLock::new();

    // Initialize battery service
    pub async fn init() {
        let battery_service = BATTERY_SERVICE.get_or_init(|| Service::new());

        comms::register_endpoint(battery_service, &battery_service.endpoint)
            .await
            .unwrap();
    }

    // Service to update the battery value in the memory map periodically
    #[embassy_executor::task]
    pub async fn battery_update_service() {
        let battery_service = BATTERY_SERVICE.get().await;

        let mut battery_remain_cap = u32::max_value();

        loop {
            battery_service
                .endpoint
                .send(
                    EndpointID::External(External::Host),
                    &espi_service::Message::BatRemainCap(battery_remain_cap),
                )
                .await
                .unwrap();
            info!("Sending updated battery status to espi service");
            battery_remain_cap -= 1;

            embassy_time::Timer::after_secs(1).await;
        }
    }

    // Service to receive battery configuration request from the host
    #[embassy_executor::task]
    pub async fn battery_config_service() {
        let battery_service = BATTERY_SERVICE.get().await;

        loop {
            let msg = battery_service.signal.wait().await;

            match msg {
                Message::BatSampleTime(sample_time) => {
                    info!("Battery Sample Time {}", sample_time);
                }
                _ => {
                    info!("Unknown message received");
                }
            }
        }
    }
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let _p = embassy_imxrt::init(Default::default());

    info!("Platform initialization complete ...");

    embedded_services::init().await;

    info!("Service initialization complete...");

    espi_service::init().await;
    battery_service::init().await;

    spawner.spawn(battery_service::battery_update_service()).unwrap();
    spawner.spawn(battery_service::battery_config_service()).unwrap();

    info!("Subsystem initialization complete...");

    // Pretend this is a host sending a message to update the battery sample time every second
    loop {
        comms::send(
            comms::EndpointID::External(comms::External::Host),
            comms::EndpointID::Internal(comms::Internal::Battery),
            &Message::BatSampleTime(10),
        )
        .await
        .unwrap();
        embassy_time::Timer::after_secs(1).await;
    }
}
