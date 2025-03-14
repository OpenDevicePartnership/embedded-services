#![no_std]
#![no_main]

extern crate rt633_examples;

use core::slice::{self};

use bq25773::Bq25773;
use bq40z50::Bq40z50;
use defmt::info;
use embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice;
use embassy_executor::Spawner;
use embassy_imxrt::bind_interrupts;
use embassy_imxrt::espi::{Base, Capabilities, Config, Direction, Espi, InterruptHandler, Len, Maxspd, PortConfig};
use embassy_imxrt::peripherals::ESPI;
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::mutex::Mutex;
use embassy_sync::once_lock::OnceLock;
use static_cell::StaticCell;
use {defmt_rtt as _, panic_probe as _};

bind_interrupts!(struct IrqsChg {
    FLEXCOMM2 => embassy_imxrt::i2c::InterruptHandler<embassy_imxrt::peripherals::FLEXCOMM2>;
});
bind_interrupts!(struct IrqsFg {
    FLEXCOMM7 => embassy_imxrt::i2c::InterruptHandler<embassy_imxrt::peripherals::FLEXCOMM7>;
});

battery_service::create_battery_service!(
    Bq25773,
    I2cDevice<'static, NoopRawMutex, embassy_imxrt::i2c::master::I2cMaster<'_, embassy_imxrt::i2c::Async>>,
    Bq40z50,
    I2cDevice<'static, NoopRawMutex, embassy_imxrt::i2c::master::I2cMaster<'_, embassy_imxrt::i2c::Async>>
);

static I2C_BUS_CHG: StaticCell<
    Mutex<NoopRawMutex, embassy_imxrt::i2c::master::I2cMaster<'_, embassy_imxrt::i2c::Async>>,
> = StaticCell::new();
static I2C_BUS_FG: StaticCell<
    Mutex<NoopRawMutex, embassy_imxrt::i2c::master::I2cMaster<'_, embassy_imxrt::i2c::Async>>,
> = StaticCell::new();

bind_interrupts!(struct Irqs {
    ESPI => InterruptHandler<ESPI>;
});

extern "C" {
    static __start_espi_data: u8;
    static __end_espi_data: u8;
}

#[embassy_executor::task]
async fn battery_timer_callback() {
    let s = SERVICE.get().await;
    loop {
        info!("battery broadcast");
        s.broadcast_dynamic_battery_msgs(&[
            battery_service::BatteryMsgs::Acpi(embedded_services::ec_type::message::BatteryMessage::CycleCount(0)),
            battery_service::BatteryMsgs::Acpi(embedded_services::ec_type::message::BatteryMessage::State(0)),
            battery_service::BatteryMsgs::Acpi(embedded_services::ec_type::message::BatteryMessage::PresentRate(0)),
            battery_service::BatteryMsgs::Acpi(embedded_services::ec_type::message::BatteryMessage::RemainCap(0)),
            battery_service::BatteryMsgs::Acpi(embedded_services::ec_type::message::BatteryMessage::PresentVolt(0)),
        ])
        .await;
        embassy_time::Timer::after_secs(1).await;
    }
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
                    addr: 0,
                    offset: 0,
                    length: Len::Len512,
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

    let i2c_chg = embassy_imxrt::i2c::master::I2cMaster::new_async(
        p.FLEXCOMM2,
        p.PIO0_18,
        p.PIO0_17,
        IrqsChg,
        embassy_imxrt::i2c::master::Speed::Standard,
        p.DMA0_CH5,
    )
    .unwrap();

    let i2c_fg = embassy_imxrt::i2c::master::I2cMaster::new_async(
        p.FLEXCOMM7,
        p.PIO4_1,
        p.PIO4_2,
        IrqsFg,
        embassy_imxrt::i2c::master::Speed::Standard,
        p.DMA0_CH15,
    )
    .unwrap();

    let i2c_bus = Mutex::new(i2c_chg);
    let i2c_bus = I2C_BUS_CHG.init(i2c_bus);

    let i2c_bus_fg = Mutex::new(i2c_fg);
    let i2c_bus_fg = I2C_BUS_FG.init(i2c_bus_fg);

    let chg_bus = I2cDevice::new(i2c_bus);
    let fg_bus = I2cDevice::new(i2c_bus_fg);

    battery_service_init(chg_bus, fg_bus).await;

    spawner.spawn(battery_service_task(spawner)).unwrap();
    spawner.spawn(battery_timer_callback()).unwrap();

    loop {
        embassy_time::Timer::after_secs(10).await;
        info!("The uptime is {} secs", embassy_time::Instant::now().as_secs());

        let data = unsafe {
            let start_espi_data = &__start_espi_data as *const u8 as *mut u8;
            let end_espi_data = &__end_espi_data as *const u8 as *mut u8;
            let espi_data_len = end_espi_data.offset_from(start_espi_data) as usize;

            slice::from_raw_parts_mut(start_espi_data, espi_data_len)
        };

        info!("Memory map contents: {:?}", data[..64]);
    }
}
