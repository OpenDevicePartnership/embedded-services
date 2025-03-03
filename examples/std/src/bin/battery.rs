use std::convert::Infallible;

use embassy_executor::{Executor, Spawner};
use embassy_sync::once_lock::OnceLock;
use embedded_batteries_async::{
    charger::{MilliAmps, MilliVolts},
    smart_battery::{
        BatteryModeFields, BatteryStatusFields, CapacityModeSignedValue, CapacityModeValue, DeciKelvin,
        ManufactureDate, Minutes, SpecificationInfoFields,
    },
};
use embedded_services::ec_type::message::BatteryMessage;
use log::*;

use embedded_hal_mock::eh1::i2c::{Mock, Transaction};

use static_cell::StaticCell;

mod espi_service {
    use embassy_sync::{blocking_mutex::raw::NoopRawMutex, once_lock::OnceLock, signal::Signal};
    use embedded_services::{
        comms::{self, EndpointID, External},
        ec_type::message::BatteryMessage,
    };
    use log::info;

    pub struct Service {
        endpoint: comms::Endpoint,
        signal: Signal<NoopRawMutex, BatteryMessage>,
    }

    impl Service {
        pub fn new() -> Self {
            Service {
                endpoint: comms::Endpoint::uninit(EndpointID::External(External::Host)),
                signal: Signal::new(),
            }
        }
    }

    impl comms::MailboxDelegate for Service {
        fn receive(&self, message: &comms::Message) {
            if let Some(msg) = message.data.get::<BatteryMessage>() {
                match msg {
                    BatteryMessage::CycleCount(cycles) => info!("Bat cycles: {}", cycles),
                    _ => todo!(),
                }
            }
        }
    }

    static ESPI_SERVICE: OnceLock<Service> = OnceLock::new();

    pub async fn init() {
        let espi_service = ESPI_SERVICE.get_or_init(|| Service::new());

        comms::register_endpoint(espi_service, &espi_service.endpoint)
            .await
            .unwrap();
    }
}

struct MockCharger<I2c: embedded_hal_async::i2c::I2c> {
    mock_bus: I2c,
}

impl<I2c: embedded_hal_async::i2c::I2c> MockCharger<I2c> {
    pub fn new(i2c: I2c) -> Self {
        MockCharger { mock_bus: i2c }
    }
}

impl<I2c: embedded_hal_async::i2c::I2c> embedded_batteries_async::charger::ErrorType for MockCharger<I2c> {
    type Error = Infallible;
}

impl<I2c: embedded_hal_async::i2c::I2c> embedded_batteries_async::charger::Charger for MockCharger<I2c> {
    async fn charging_current(&mut self, current: MilliAmps) -> Result<MilliAmps, Self::Error> {
        Ok(0)
    }

    async fn charging_voltage(&mut self, voltage: MilliVolts) -> Result<MilliVolts, Self::Error> {
        Ok(0)
    }
}

struct MockFuelGauge<I2c: embedded_hal_async::i2c::I2c> {
    mock_bus: I2c,
}

impl<I2c: embedded_hal_async::i2c::I2c> MockFuelGauge<I2c> {
    pub fn new(i2c: I2c) -> Self {
        MockFuelGauge { mock_bus: i2c }
    }
}

impl<I2c: embedded_hal_async::i2c::I2c> embedded_batteries_async::smart_battery::ErrorType for MockFuelGauge<I2c> {
    type Error = Infallible;
}

impl<I2c: embedded_hal_async::i2c::I2c> embedded_batteries_async::smart_battery::SmartBattery for MockFuelGauge<I2c> {
    async fn remaining_capacity_alarm(&mut self) -> Result<CapacityModeValue, Self::Error> {
        Ok(CapacityModeValue::MilliAmpUnsigned(0))
    }

    async fn set_remaining_capacity_alarm(&mut self, capacity: CapacityModeValue) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn remaining_time_alarm(&mut self) -> Result<Minutes, Self::Error> {
        Ok(0)
    }

    async fn set_remaining_time_alarm(&mut self, time: Minutes) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn battery_mode(&mut self) -> Result<BatteryModeFields, Self::Error> {
        Ok(BatteryModeFields::new())
    }

    async fn set_battery_mode(&mut self, flags: BatteryModeFields) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn at_rate(&mut self) -> Result<CapacityModeSignedValue, Self::Error> {
        Ok(CapacityModeSignedValue::MilliAmpSigned(0))
    }

    async fn set_at_rate(&mut self, rate: CapacityModeSignedValue) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn at_rate_time_to_full(&mut self) -> Result<embedded_batteries_async::smart_battery::Minutes, Self::Error> {
        Ok(0)
    }

    async fn at_rate_time_to_empty(&mut self) -> Result<embedded_batteries_async::smart_battery::Minutes, Self::Error> {
        Ok(0)
    }

    async fn at_rate_ok(&mut self) -> Result<bool, Self::Error> {
        Ok(true)
    }

    async fn temperature(&mut self) -> Result<DeciKelvin, Self::Error> {
        Ok(0)
    }

    async fn voltage(&mut self) -> Result<MilliVolts, Self::Error> {
        Ok(0)
    }

    async fn current(&mut self) -> Result<embedded_batteries_async::smart_battery::MilliAmpsSigned, Self::Error> {
        Ok(0)
    }

    async fn average_current(
        &mut self,
    ) -> Result<embedded_batteries_async::smart_battery::MilliAmpsSigned, Self::Error> {
        Ok(0)
    }

    async fn max_error(&mut self) -> Result<embedded_batteries_async::smart_battery::Percent, Self::Error> {
        Ok(0)
    }

    async fn relative_state_of_charge(
        &mut self,
    ) -> Result<embedded_batteries_async::smart_battery::Percent, Self::Error> {
        Ok(0)
    }

    async fn absolute_state_of_charge(
        &mut self,
    ) -> Result<embedded_batteries_async::smart_battery::Percent, Self::Error> {
        Ok(0)
    }

    async fn remaining_capacity(
        &mut self,
    ) -> Result<embedded_batteries_async::smart_battery::CapacityModeValue, Self::Error> {
        Ok(CapacityModeValue::MilliAmpUnsigned(0))
    }

    async fn full_charge_capacity(
        &mut self,
    ) -> Result<embedded_batteries_async::smart_battery::CapacityModeValue, Self::Error> {
        Ok(CapacityModeValue::MilliAmpUnsigned(0))
    }

    async fn run_time_to_empty(&mut self) -> Result<embedded_batteries_async::smart_battery::Minutes, Self::Error> {
        Ok(0)
    }

    async fn average_time_to_empty(&mut self) -> Result<embedded_batteries_async::smart_battery::Minutes, Self::Error> {
        Ok(0)
    }

    async fn average_time_to_full(&mut self) -> Result<embedded_batteries_async::smart_battery::Minutes, Self::Error> {
        Ok(0)
    }

    async fn battery_status(
        &mut self,
    ) -> Result<embedded_batteries_async::smart_battery::BatteryStatusFields, Self::Error> {
        Ok(BatteryStatusFields::new())
    }

    async fn cycle_count(&mut self) -> Result<embedded_batteries_async::smart_battery::Cycles, Self::Error> {
        Ok(33)
    }

    async fn design_capacity(
        &mut self,
    ) -> Result<embedded_batteries_async::smart_battery::CapacityModeValue, Self::Error> {
        Ok(CapacityModeValue::MilliAmpUnsigned(0))
    }

    async fn design_voltage(&mut self) -> Result<MilliVolts, Self::Error> {
        Ok(0)
    }

    async fn specification_info(&mut self) -> Result<SpecificationInfoFields, Self::Error> {
        Ok(SpecificationInfoFields::new())
    }

    async fn manufacture_date(
        &mut self,
    ) -> Result<embedded_batteries_async::smart_battery::ManufactureDate, Self::Error> {
        Ok(ManufactureDate::new())
    }

    async fn serial_number(&mut self) -> Result<u16, Self::Error> {
        Ok(0)
    }

    async fn manufacturer_name(&mut self, name: &mut [u8]) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn device_name(&mut self, name: &mut [u8]) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn device_chemistry(&mut self, chemistry: &mut [u8]) -> Result<(), Self::Error> {
        Ok(())
    }
}

battery_service::create_battery_service!(
    MockCharger,
    embedded_hal_mock::common::Generic<Transaction>,
    MockFuelGauge,
    embedded_hal_mock::common::Generic<Transaction>
);

#[embassy_executor::task]
async fn init_task() {
    embedded_services::init().await;
    info!("services init'd");

    espi_service::init().await;
    info!("espi service init'd");

    let expectations = vec![];
    let chg_i2c = Mock::new(&expectations);
    let fg_i2c = Mock::new(&expectations);

    battery_service_init(chg_i2c, fg_i2c).await;
    info!("battery service init'd");
}

#[embassy_executor::task]
async fn battery_timer_callback() {
    let s = SERVICE.get().await;
    let mut cycle_count = 0;
    loop {
        info!("battery broadcast");
        s.broadcast_dynamic_acpi_msgs(&[BatteryMessage::CycleCount(cycle_count)])
            .await;
        cycle_count += 1;
        embassy_time::Timer::after_secs(1).await;
    }
}

fn main() {
    env_logger::builder().filter_level(log::LevelFilter::Info).init();

    static EXECUTOR: StaticCell<Executor> = StaticCell::new();
    let executor = EXECUTOR.init(Executor::new());

    executor.run(|spawner| {
        spawner.must_spawn(init_task());
        spawner.must_spawn(battery_service_task(spawner));
        spawner.must_spawn(battery_timer_callback());
    });
}
