#![no_std]
#![no_main]

extern crate rt685s_evk_example;

use defmt::info;
use embassy_executor::Spawner;

#[derive(Copy, Clone, Debug, defmt::Format)]
enum TxMessage {
    UpdateBatteryStatus(u32),
}

#[derive(Copy, Clone, Debug, defmt::Format)]
enum RxMessage {
    SetBatteryCharge(u32),
}

// Mock eSPI transport service
mod espi_service {
    use crate::{RxMessage, TxMessage};
    use core::convert::Infallible;
    use defmt::info;
    use embassy_sync::blocking_mutex::raw::NoopRawMutex;
    use embassy_sync::once_lock::OnceLock;
    use embassy_sync::signal::Signal;
    use embedded_services::comms::{self, EndpointID, External, Internal};

    struct Service {
        endpoint: comms::Endpoint,

        // This is can be an Embassy signal or channel or whatever Embassy async notification construct
        signal: Signal<NoopRawMutex, TxMessage>,
    }

    impl Service {
        fn new() -> Self {
            Service {
                endpoint: comms::Endpoint::uninit(EndpointID::External(External::Host)),
                signal: Signal::new(),
            }
        }
    }

    impl comms::MailboxDelegate for Service {
        fn receive(&self, message: &comms::Message) -> Result<(), comms::MailboxDelegateError> {
            let msg = message
                .data
                .get::<TxMessage>()
                .ok_or(comms::MailboxDelegateError::MessageNotFound)?;

            self.signal.signal(*msg);

            Ok(())
        }
    }

    static ESPI_SERVICE: OnceLock<Service> = OnceLock::new();

    // Initialize eSPI service and register it with the transport service
    pub async fn init() {
        let espi_service = ESPI_SERVICE.get_or_init(Service::new);

        comms::register_endpoint(espi_service, &espi_service.endpoint)
            .await
            .unwrap();
    }

    // Funtion to forward a battery_charge_message to the battery service
    pub async fn forward_set_battery_charge_message(battery_charge: u32) -> Result<(), Infallible> {
        let espi_service = ESPI_SERVICE.get().await;

        espi_service
            .endpoint
            .send(
                EndpointID::Internal(Internal::Battery),
                &RxMessage::SetBatteryCharge(battery_charge),
            )
            .await
    }

    // espi service that will update the memory map
    #[embassy_executor::task]
    pub async fn espi_service() {
        let espi_service = ESPI_SERVICE.get().await;

        loop {
            let msg = espi_service.signal.wait().await;

            match msg {
                TxMessage::UpdateBatteryStatus(_) => {
                    info!("Update battery status in memory map");
                    embassy_time::Timer::after_secs(1).await;
                }
            }
        }
    }
}

// Mock battery service
mod battery_service {
    use crate::{RxMessage, TxMessage};
    use defmt::info;
    use embassy_sync::blocking_mutex::raw::NoopRawMutex;
    use embassy_sync::once_lock::OnceLock;
    use embassy_sync::signal::Signal;
    use embedded_services::comms::{self, EndpointID, External, Internal};

    struct Service {
        endpoint: comms::Endpoint,

        // This is can be an Embassy signal or channel or whatever Embassy async notification construct
        signal: Signal<NoopRawMutex, RxMessage>,
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
        fn receive(&self, message: &comms::Message) -> Result<(), comms::MailboxDelegateError> {
            let msg = message
                .data
                .get::<RxMessage>()
                .ok_or(comms::MailboxDelegateError::MessageNotFound)?;

            self.signal.signal(*msg);

            Ok(())
        }
    }

    static BATTERY_SERVICE: OnceLock<Service> = OnceLock::new();

    // Initialize battery service
    pub async fn init() {
        let battery_service = BATTERY_SERVICE.get_or_init(Service::new);

        comms::register_endpoint(battery_service, &battery_service.endpoint)
            .await
            .unwrap();
    }

    // Service to update the battery value in the memory map periodically
    #[embassy_executor::task]
    pub async fn battery_update_service() {
        let battery_service = BATTERY_SERVICE.get().await;

        loop {
            let battery_status = 0;
            battery_service
                .endpoint
                .send(
                    EndpointID::External(External::Host),
                    &TxMessage::UpdateBatteryStatus(battery_status),
                )
                .await
                .unwrap();
            info!("Sending updated battery status to espi service");

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
                RxMessage::SetBatteryCharge(charge) => {
                    info!("Set battery charge {}", charge);
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

    spawner.spawn(espi_service::espi_service()).unwrap();

    spawner.spawn(battery_service::battery_update_service()).unwrap();
    spawner.spawn(battery_service::battery_config_service()).unwrap();

    info!("Subsystem initialization complete...");

    // Pretend this loop is an interrupt that fires every second to set the battery charge
    let mut battery_charge = 0;
    loop {
        battery_charge += 1;
        embassy_time::Timer::after_secs(1).await;
        espi_service::forward_set_battery_charge_message(battery_charge)
            .await
            .unwrap();
    }
}
