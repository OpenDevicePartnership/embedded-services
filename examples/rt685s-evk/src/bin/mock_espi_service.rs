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
    use defmt::info;
    use embassy_futures::select::{Either, select};
    use embassy_sync::blocking_mutex::raw::NoopRawMutex;
    use embassy_sync::signal::Signal;
    use embassy_time::{Duration, Ticker};
    use embedded_services::comms::{self, EndpointID, External, Internal};
    use static_cell::StaticCell;

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

    // espi service that will update the memory map
    #[embassy_executor::task]
    pub async fn espi_service() {
        static ESPI_SERVICE: StaticCell<Service> = StaticCell::new();
        let espi_service = ESPI_SERVICE.init(Service::new());
        let mut ticker = Ticker::every(Duration::from_secs(1));

        comms::register_endpoint(espi_service, &espi_service.endpoint)
            .await
            .unwrap();

        let mut battery_charge = 0;

        loop {
            let event = select(espi_service.signal.wait(), ticker.next()).await;

            match event {
                Either::First(msg) => match msg {
                    TxMessage::UpdateBatteryStatus(charge) => {
                        info!("Update battery charge: {}", charge);
                        battery_charge = charge;
                        embassy_time::Timer::after_secs(1).await;
                    }
                },
                Either::Second(_) => {
                    espi_service
                        .endpoint
                        .send(
                            EndpointID::Internal(Internal::Battery),
                            &RxMessage::SetBatteryCharge(battery_charge),
                        )
                        .await
                        .unwrap();
                }
            }
        }
    }
}

// Mock battery service
mod battery_service {
    use crate::{RxMessage, TxMessage};
    use defmt::info;
    use embassy_futures::select::{Either, select};
    use embassy_sync::blocking_mutex::raw::NoopRawMutex;
    use embassy_sync::signal::Signal;
    use embassy_time::{Duration, Ticker};
    use embedded_services::comms::{self, EndpointID, External, Internal};
    use static_cell::StaticCell;

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

    // Service to receive battery configuration request from the host
    #[embassy_executor::task]
    pub async fn battery_service_task() {
        static BATTERY_SERVICE: StaticCell<Service> = StaticCell::new();
        let battery_service = BATTERY_SERVICE.init(Service::new());
        let mut ticker = Ticker::every(Duration::from_secs(1));

        comms::register_endpoint(battery_service, &battery_service.endpoint)
            .await
            .unwrap();

        loop {
            let event = select(battery_service.signal.wait(), ticker.next()).await;

            match event {
                Either::First(msg) => match msg {
                    RxMessage::SetBatteryCharge(charge) => {
                        info!("Set battery charge {}", charge);
                    }
                },
                Either::Second(_) => {
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

    spawner.spawn(espi_service::espi_service()).unwrap();

    spawner.spawn(battery_service::battery_service_task()).unwrap();

    info!("Subsystem initialization complete...");
}
