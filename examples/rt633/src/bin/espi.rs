#![no_std]
#![no_main]

extern crate rt633_examples;

use core::slice::{self};

use defmt::info;
use embassy_executor::Spawner;
use embassy_imxrt::bind_interrupts;
use embassy_imxrt::espi::BaseOrAsz;
use embassy_imxrt::espi::{Base, Capabilities, Config, Direction, Espi, InterruptHandler, Len, Maxspd, PortConfig};
use embassy_imxrt::peripherals::ESPI;
use {defmt_rtt as _, panic_probe as _};

// Mock battery service
mod battery_service {
    use defmt::info;
    use embassy_sync::blocking_mutex::raw::NoopRawMutex;
    use embassy_sync::once_lock::OnceLock;
    use embassy_sync::signal::Signal;
    use embedded_services::comms::{self, EndpointID, External, Internal};
    use embedded_services::ec_type;

    struct Service {
        endpoint: comms::Endpoint,

        // This is can be an Embassy signal or channel or whatever Embassy async notification construct
        signal: Signal<NoopRawMutex, ec_type::message::BatteryMessage>,
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
                .get::<ec_type::message::BatteryMessage>()
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

        let mut battery_remain_cap = u32::MAX;

        loop {
            battery_service
                .endpoint
                .send(
                    EndpointID::External(External::Host),
                    &ec_type::message::BatteryMessage::RemainCap(battery_remain_cap),
                )
                .await
                .unwrap();
            info!("Sending updated battery status to espi service");
            battery_remain_cap -= 1;

            embassy_time::Timer::after_secs(1).await;
        }
    }
}

bind_interrupts!(struct Irqs {
    ESPI => InterruptHandler<ESPI>;
});

extern "C" {
    static __start_espi_data: u8;
    static __end_espi_data: u8;
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_imxrt::init(Default::default());

    embedded_services::init().await;

    let espi = Espi::new(
        p.ESPI,
        p.PIO7_29,
        p.PIO7_26,
        p.PIO7_27,
        p.PIO7_28,
        p.PIO7_30,
        p.PIO7_31,
        p.PIO7_25,
        p.PIO7_24,
        Irqs,
        Config {
            caps: Capabilities {
                max_speed: Maxspd::SmallThan20m,
                alert_as_a_pin: true,
                ..Default::default()
            },
            ram_base: 0x2000_0000,
            base0_addr: 0x2002_0000,
            base1_addr: 0x2003_0000,
            status_addr: Some(0x480),
            status_base: Base::OffsetFrom0,
            ports_config: [
                PortConfig::MailboxShared {
                    direction: Direction::BidirectionalUnenforced,
                    base_sel: BaseOrAsz::OffsetFrom0,
                    offset: 0,
                    length: Len::Len256,
                },
                Default::default(),
                Default::default(),
                Default::default(),
                Default::default(),
            ],
            ..Default::default()
        },
    );

    let memory_map_buffer = unsafe {
        let start_espi_data = &__start_espi_data as *const u8 as *mut u8;
        let end_espi_data = &__end_espi_data as *const u8 as *mut u8;
        let espi_data_len = end_espi_data.offset_from(start_espi_data) as usize;

        slice::from_raw_parts_mut(start_espi_data, espi_data_len)
    };

    spawner.must_spawn(espi_service::espi_service(espi, memory_map_buffer));

    battery_service::init().await;

    spawner.spawn(battery_service::battery_update_service()).unwrap();

    loop {
        embassy_time::Timer::after_secs(10).await;
        info!("The uptime is {} secs", embassy_time::Instant::now().as_secs());

        let data = unsafe {
            let start_espi_data = &__start_espi_data as *const u8 as *mut u8;
            let end_espi_data = &__end_espi_data as *const u8 as *mut u8;
            let espi_data_len = end_espi_data.offset_from(start_espi_data) as usize;

            slice::from_raw_parts_mut(start_espi_data, espi_data_len)
        };

        info!("Memory map contents: {:?}", data[..256]);
    }
}
