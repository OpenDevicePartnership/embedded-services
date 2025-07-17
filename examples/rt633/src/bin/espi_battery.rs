#![no_std]
#![no_main]

extern crate rt633_examples;

use battery_service::context::BatteryEvent;
use core::slice::{self};
use embassy_imxrt::dma::NoDma;
use embassy_time::{Duration, Timer};
use embedded_batteries_async::charger::MilliAmps;
use embedded_batteries_async::smart_battery::{
    BatteryModeFields, BatteryStatusFields, CapacityModeSignedValue, CapacityModeValue, Cycles, DeciKelvin,
    ManufactureDate, MilliAmpsSigned, MilliVolts, Minutes, Percent, SmartBattery, SpecificationInfoFields,
};
use embedded_services::{error, info};

use battery_service::controller::{Controller, ControllerEvent};
use battery_service::device::{Device, DeviceId, DynamicBatteryMsgs, StaticBatteryMsgs};
use battery_service::wrapper::Wrapper;
use bq40z50_rx::Bq40z50;
use embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice;
use embassy_executor::Spawner;
use embassy_imxrt::bind_interrupts;
use embassy_imxrt::espi::BaseOrAsz;
use embassy_imxrt::espi::{Base, Capabilities, Config, Direction, Espi, InterruptHandler, Len, Maxspd, PortConfig};
use embassy_imxrt::i2c::Async;
use embassy_imxrt::i2c::master::I2cMaster;
use embassy_imxrt::peripherals::ESPI;
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::mutex::Mutex;
use static_cell::StaticCell;
use {defmt_rtt as _, panic_probe as _};

bind_interrupts!(struct IrqsFg {
    FLEXCOMM15 => embassy_imxrt::i2c::InterruptHandler<embassy_imxrt::peripherals::FLEXCOMM15>;
});

static I2C_BUS_FG: StaticCell<
    Mutex<NoopRawMutex, embassy_imxrt::i2c::master::I2cMaster<'_, embassy_imxrt::i2c::Async>>,
> = StaticCell::new();
static FG_DEVICE: StaticCell<Device> = StaticCell::new();

/// Wrapper struct for the fuel gauge driver
struct Bq40z50Controller {
    driver: Bq40z50<
        I2cDevice<'static, NoopRawMutex, embassy_imxrt::i2c::master::I2cMaster<'static, embassy_imxrt::i2c::Async>>,
    >,
}

impl embedded_batteries_async::smart_battery::ErrorType for Bq40z50Controller {
    type Error = <Bq40z50<I2cDevice<'static, NoopRawMutex, I2cMaster<'static, Async>>> as embedded_batteries_async::smart_battery::ErrorType>::Error;
}

impl embedded_batteries_async::smart_battery::SmartBattery for Bq40z50Controller {
    async fn absolute_state_of_charge(&mut self) -> Result<Percent, Self::Error> {
        self.driver.absolute_state_of_charge().await
    }
    async fn at_rate(&mut self) -> Result<CapacityModeSignedValue, Self::Error> {
        self.driver.at_rate().await
    }
    async fn at_rate_ok(&mut self) -> Result<bool, Self::Error> {
        self.driver.at_rate_ok().await
    }
    async fn at_rate_time_to_empty(&mut self) -> Result<Minutes, Self::Error> {
        self.driver.at_rate_time_to_empty().await
    }
    async fn at_rate_time_to_full(&mut self) -> Result<Minutes, Self::Error> {
        self.driver.at_rate_time_to_full().await
    }
    async fn average_current(&mut self) -> Result<MilliAmpsSigned, Self::Error> {
        self.driver.average_current().await
    }
    async fn average_time_to_empty(&mut self) -> Result<Minutes, Self::Error> {
        self.driver.average_time_to_empty().await
    }
    async fn average_time_to_full(&mut self) -> Result<Minutes, Self::Error> {
        self.driver.average_time_to_full().await
    }
    async fn battery_mode(&mut self) -> Result<BatteryModeFields, Self::Error> {
        self.driver.battery_mode().await
    }
    async fn battery_status(&mut self) -> Result<BatteryStatusFields, Self::Error> {
        self.driver.battery_status().await
    }
    async fn current(&mut self) -> Result<MilliAmpsSigned, Self::Error> {
        self.driver.current().await
    }
    async fn cycle_count(&mut self) -> Result<Cycles, Self::Error> {
        self.driver.cycle_count().await
    }
    async fn design_capacity(&mut self) -> Result<CapacityModeValue, Self::Error> {
        self.driver.design_capacity().await
    }
    async fn design_voltage(&mut self) -> Result<MilliVolts, Self::Error> {
        self.driver.design_voltage().await
    }
    async fn device_chemistry(&mut self, chemistry: &mut [u8]) -> Result<(), Self::Error> {
        self.driver.device_chemistry(chemistry).await
    }
    async fn device_name(&mut self, name: &mut [u8]) -> Result<(), Self::Error> {
        self.driver.device_name(name).await
    }
    async fn full_charge_capacity(&mut self) -> Result<CapacityModeValue, Self::Error> {
        self.driver.full_charge_capacity().await
    }
    async fn manufacture_date(&mut self) -> Result<ManufactureDate, Self::Error> {
        self.driver.manufacture_date().await
    }
    async fn manufacturer_name(&mut self, name: &mut [u8]) -> Result<(), Self::Error> {
        self.driver.manufacturer_name(name).await
    }
    async fn max_error(&mut self) -> Result<Percent, Self::Error> {
        self.driver.max_error().await
    }
    async fn relative_state_of_charge(&mut self) -> Result<Percent, Self::Error> {
        self.driver.relative_state_of_charge().await
    }
    async fn remaining_capacity(&mut self) -> Result<CapacityModeValue, Self::Error> {
        self.driver.remaining_capacity().await
    }
    async fn remaining_capacity_alarm(&mut self) -> Result<CapacityModeValue, Self::Error> {
        self.driver.remaining_capacity_alarm().await
    }
    async fn remaining_time_alarm(&mut self) -> Result<Minutes, Self::Error> {
        self.driver.remaining_time_alarm().await
    }
    async fn run_time_to_empty(&mut self) -> Result<Minutes, Self::Error> {
        self.driver.run_time_to_empty().await
    }
    async fn serial_number(&mut self) -> Result<u16, Self::Error> {
        self.driver.serial_number().await
    }
    async fn set_at_rate(&mut self, rate: CapacityModeSignedValue) -> Result<(), Self::Error> {
        self.driver.set_at_rate(rate).await
    }
    async fn set_battery_mode(&mut self, flags: BatteryModeFields) -> Result<(), Self::Error> {
        self.driver.set_battery_mode(flags).await
    }
    async fn set_remaining_capacity_alarm(&mut self, capacity: CapacityModeValue) -> Result<(), Self::Error> {
        self.driver.set_remaining_capacity_alarm(capacity).await
    }
    async fn set_remaining_time_alarm(&mut self, time: Minutes) -> Result<(), Self::Error> {
        self.driver.set_remaining_time_alarm(time).await
    }
    async fn specification_info(&mut self) -> Result<SpecificationInfoFields, Self::Error> {
        self.driver.specification_info().await
    }
    async fn temperature(&mut self) -> Result<DeciKelvin, Self::Error> {
        self.driver.temperature().await
    }
    async fn voltage(&mut self) -> Result<MilliVolts, Self::Error> {
        self.driver.voltage().await
    }
    async fn charging_current(&mut self) -> Result<MilliAmps, Self::Error> {
        self.driver.charging_current().await
    }
    async fn charging_voltage(&mut self) -> Result<MilliVolts, Self::Error> {
        self.driver.charging_voltage().await
    }
}

impl Controller for Bq40z50Controller {
    type ControllerError = <Bq40z50<I2cDevice<'static, NoopRawMutex, I2cMaster<'static, Async>>> as embedded_batteries_async::smart_battery::ErrorType>::Error;

    async fn initialize(&mut self) -> Result<(), Self::ControllerError> {
        info!("Fuel gauge inited!");
        Ok(())
    }

    async fn get_static_data(&mut self) -> Result<StaticBatteryMsgs, Self::ControllerError> {
        info!("Sending static data");

        Ok(StaticBatteryMsgs { ..Default::default() })
    }

    async fn get_dynamic_data(&mut self) -> Result<DynamicBatteryMsgs, Self::ControllerError> {
        info!("Sending dynamic data");
        info!("Voltage = {}", self.voltage().await?);
        info!("Current = {}", self.current().await?);
        info!("Cycle count = {}", self.cycle_count().await?);

        Ok(DynamicBatteryMsgs { ..Default::default() })
    }

    async fn get_device_event(&mut self) -> ControllerEvent {
        loop {
            Timer::after_secs(1000000).await;
        }
    }

    async fn ping(&mut self) -> Result<(), Self::ControllerError> {
        info!("Ping!");
        Ok(())
    }

    fn get_timeout(&self) -> Duration {
        unimplemented!()
    }

    fn set_timeout(&mut self, _duration: Duration) {
        unimplemented!()
    }
}

bind_interrupts!(struct Irqs {
    ESPI => InterruptHandler<ESPI>;
});

// SAFETY: These are symbols defined by the linker and guaranteed to point to valid memory
unsafe extern "C" {
    static __start_espi_data: u8;
    static __end_espi_data: u8;
}

#[embassy_executor::task]
async fn battery_publish_task(fg_device: &'static Device) {
    loop {
        Timer::after_secs(1).await;
        // Get dynamic cache
        let cache = fg_device.get_dynamic_battery_cache().await;

        // Send cache data to eSpi service
        battery_service::comms_send(
            embedded_services::comms::EndpointID::External(embedded_services::comms::External::Host),
            &embedded_services::ec_type::message::BatteryMessage::CycleCount(cache.cycle_count.into()),
        )
        .await
        .unwrap();
    }
}

#[embassy_executor::task]
async fn wrapper_task(wrapper: Wrapper<'static, Bq40z50Controller>) {
    loop {
        wrapper.process().await;
        info!("Got new wrapper message");
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
                    base_sel: BaseOrAsz::OffsetFrom0,
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

    let config = embassy_imxrt::i2c::master::Config {
        speed: embassy_imxrt::i2c::master::Speed::Standard,
        duty_cycle: embassy_imxrt::i2c::master::DutyCycle::new(50).unwrap(),
    };

    let i2c_fg = embassy_imxrt::i2c::master::I2cMaster::new_async(
        p.FLEXCOMM15,
        p.PIOFC15_SCL,
        p.PIOFC15_SDA,
        IrqsFg,
        config,
        unsafe { embassy_imxrt::Peri::new_unchecked(NoDma) },
    )
    .unwrap();

    let i2c_bus_fg = I2C_BUS_FG.init(Mutex::new(i2c_fg));

    let fg_bus = I2cDevice::new(i2c_bus_fg);

    let fg = FG_DEVICE.init(Device::new(DeviceId(0)));

    let wrap = Wrapper::new(
        fg,
        Bq40z50Controller {
            driver: Bq40z50::new(fg_bus),
        },
    );

    spawner.must_spawn(wrapper_task(wrap));
    spawner.must_spawn(battery_service::task());

    battery_service::register_fuel_gauge(fg).await.unwrap();

    spawner.must_spawn(battery_publish_task(fg));

    if let Err(e) = battery_service::execute_event(BatteryEvent {
        device_id: DeviceId(0),
        event: battery_service::context::BatteryEventInner::DoInit,
    })
    .await
    {
        error!("Error initializing fuel gauge, error: {:?}", e);
    }

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

        if let Err(e) = battery_service::execute_event(BatteryEvent {
            device_id: DeviceId(0),
            event: battery_service::context::BatteryEventInner::PollDynamicData,
        })
        .await
        {
            error!("Error getting dynamic fuel gauge data, error: {:?}", e);
        }
    }
}
