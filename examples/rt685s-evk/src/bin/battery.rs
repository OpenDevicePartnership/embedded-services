#![no_std]
#![no_main]

extern crate embedded_services_examples;

use bq25773::Bq25773;
use defmt::info;
use embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice;
use embassy_executor::Spawner;
use embassy_imxrt::bind_interrupts;
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::mutex::Mutex;
use embassy_sync::once_lock::OnceLock;
use static_cell::StaticCell;

bind_interrupts!(struct Irqs {
    FLEXCOMM2 => embassy_imxrt::i2c::InterruptHandler<embassy_imxrt::peripherals::FLEXCOMM2>;
});

battery_service::create_battery_service!(
    Bq25773,
    I2cDevice<'static, NoopRawMutex, embassy_imxrt::i2c::master::I2cMaster<'_, embassy_imxrt::i2c::Async>>
);

static I2C_BUS: StaticCell<Mutex<NoopRawMutex, embassy_imxrt::i2c::master::I2cMaster<'_, embassy_imxrt::i2c::Async>>> =
    StaticCell::new();

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_imxrt::init(Default::default());

    info!("Platform initialization complete ...");

    embedded_services::init().await;

    info!("Service initialization complete...");

    // All this can go out of scope now because these are all moves
    let i2c = embassy_imxrt::i2c::master::I2cMaster::new_async(
        p.FLEXCOMM2,
        p.PIO0_18,
        p.PIO0_17,
        Irqs,
        embassy_imxrt::i2c::master::Speed::Standard,
        p.DMA0_CH5,
    )
    .unwrap();

    let i2c_bus = Mutex::new(i2c);
    let i2c_bus = I2C_BUS.init(i2c_bus);

    let chg_bus = I2cDevice::new(i2c_bus);

    battery_service_init(chg_bus).await;

    info!("Subsystem initialization complete...");

    embassy_time::Timer::after_millis(1000).await;
}
