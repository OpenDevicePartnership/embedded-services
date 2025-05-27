use std::convert::Infallible;

use battery_service::controller::{Controller, ControllerEvent};
use battery_service::device::{Device, DeviceId, DynamicBatteryMsgs, StaticBatteryMsgs};
use battery_service::wrapper::Wrapper;
use embassy_executor::{Executor, Spawner};
use embassy_sync::once_lock::OnceLock;
use embassy_time::{Duration, Timer};
use embedded_batteries_async::charger::{MilliAmps, MilliVolts};
use embedded_batteries_async::smart_battery::{
    self, BatteryModeFields, BatteryStatusFields, CapacityModeSignedValue, CapacityModeValue, Cycles, DeciKelvin,
    ManufactureDate, MilliAmpsSigned, Minutes, Percent, SmartBattery, SpecificationInfoFields,
};
use embedded_hal_mock::eh1::i2c::Mock;
use embedded_services::info;
use static_cell::StaticCell;

mod espi_service {
    use battery_service::context::{BatteryEvent, BatteryEventInner};
    use battery_service::device::DeviceId;
    use embassy_sync::blocking_mutex::raw::NoopRawMutex;
    use embassy_sync::once_lock::OnceLock;
    use embassy_sync::signal::Signal;
    use embassy_time::Timer;
    use embedded_services::comms::{self, EndpointID, External};
    use embedded_services::ec_type::message::BatteryMessage;
    use embedded_services::error;
    use log::info;

    pub struct Service {
        endpoint: comms::Endpoint,
        _signal: Signal<NoopRawMutex, BatteryMessage>,
    }

    impl Service {
        pub fn new() -> Self {
            Service {
                endpoint: comms::Endpoint::uninit(EndpointID::External(External::Host)),
                _signal: Signal::new(),
            }
        }
    }

    impl comms::MailboxDelegate for Service {
        fn receive(&self, message: &comms::Message) -> Result<(), comms::MailboxDelegateError> {
            let msg = message
                .data
                .get::<BatteryMessage>()
                .ok_or(comms::MailboxDelegateError::MessageNotFound)?;

            match msg {
                BatteryMessage::CycleCount(cycles) => {
                    info!("Bat cycles: {}", cycles);
                    Ok(())
                }
                _ => Err(comms::MailboxDelegateError::InvalidData),
            }
        }
    }

    static ESPI_SERVICE: OnceLock<Service> = OnceLock::new();

    pub async fn init() {
        let espi_service = ESPI_SERVICE.get_or_init(Service::new);

        comms::register_endpoint(espi_service, &espi_service.endpoint)
            .await
            .unwrap();
    }

    #[embassy_executor::task]
    pub async fn task() {
        let espi_service = ESPI_SERVICE.get().await;

        espi_service
            .endpoint
            .send(
                EndpointID::Internal(comms::Internal::Battery),
                &BatteryEvent {
                    device_id: DeviceId(0),
                    event: BatteryEventInner::DoInit,
                },
            )
            .await
            .unwrap();
        info!("Sent init request");
        match battery_service::wait_for_battery_response().await {
            Ok(_) => {
                info!("Init request succeeded!")
            }
            Err(e) => {
                error!("Init request failed with {:?}", e);
            }
        }
        Timer::after_secs(5).await;

        loop {
            espi_service
                .endpoint
                .send(
                    EndpointID::Internal(comms::Internal::Battery),
                    &BatteryEvent {
                        device_id: DeviceId(0),
                        event: BatteryEventInner::PollDynamicData,
                    },
                )
                .await
                .unwrap();
            info!("Sent dynamic data request");
            match battery_service::wait_for_battery_response().await {
                Ok(_) => {
                    info!("dynamic data request succeeded!")
                }
                Err(e) => {
                    error!("dynamic data request failed with {:?}", e);
                }
            }
            Timer::after_secs(5).await;
        }
    }
}

struct FuelGaugeController {
    driver: MockFuelGaugeDriver<Mock>,
}

impl smart_battery::ErrorType for FuelGaugeController {
    type Error = Infallible;
}

impl SmartBattery for FuelGaugeController {
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
    async fn charging_current(&mut self) -> Result<MilliAmps, Self::Error> {
        self.driver.charging_current().await
    }
    async fn charging_voltage(&mut self) -> Result<MilliVolts, Self::Error> {
        self.driver.charging_voltage().await
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
}

impl Controller for FuelGaugeController {
    type ControllerError = Infallible;

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

struct MockFuelGaugeDriver<I2c: embedded_hal_async::i2c::I2c> {
    _mock_bus: I2c,
}

impl<I2c: embedded_hal_async::i2c::I2c> MockFuelGaugeDriver<I2c> {
    pub fn new(i2c: I2c) -> Self {
        MockFuelGaugeDriver { _mock_bus: i2c }
    }
}

impl<I2c: embedded_hal_async::i2c::I2c> embedded_batteries_async::smart_battery::ErrorType
    for MockFuelGaugeDriver<I2c>
{
    type Error = Infallible;
}

impl<I2c: embedded_hal_async::i2c::I2c> embedded_batteries_async::smart_battery::SmartBattery
    for MockFuelGaugeDriver<I2c>
{
    async fn remaining_capacity_alarm(&mut self) -> Result<CapacityModeValue, Self::Error> {
        Ok(CapacityModeValue::MilliAmpUnsigned(0))
    }

    async fn set_remaining_capacity_alarm(&mut self, _capacity: CapacityModeValue) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn remaining_time_alarm(&mut self) -> Result<Minutes, Self::Error> {
        Ok(0)
    }

    async fn set_remaining_time_alarm(&mut self, _time: Minutes) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn battery_mode(&mut self) -> Result<BatteryModeFields, Self::Error> {
        Ok(BatteryModeFields::new())
    }

    async fn set_battery_mode(&mut self, _flags: BatteryModeFields) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn at_rate(&mut self) -> Result<CapacityModeSignedValue, Self::Error> {
        Ok(CapacityModeSignedValue::MilliAmpSigned(0))
    }

    async fn set_at_rate(&mut self, _rate: CapacityModeSignedValue) -> Result<(), Self::Error> {
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

    async fn charging_voltage(&mut self) -> Result<MilliVolts, Self::Error> {
        Ok(0)
    }

    async fn current(&mut self) -> Result<MilliAmpsSigned, Self::Error> {
        Ok(0)
    }

    async fn charging_current(&mut self) -> Result<MilliAmps, Self::Error> {
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

    async fn manufacturer_name(&mut self, _name: &mut [u8]) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn device_name(&mut self, _name: &mut [u8]) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn device_chemistry(&mut self, _chemistry: &mut [u8]) -> Result<(), Self::Error> {
        Ok(())
    }
}

#[embassy_executor::task]
async fn init_task(spawner: Spawner, dev: &'static Device) {
    embedded_services::init().await;
    info!("services init'd");

    espi_service::init().await;
    info!("espi service init'd");

    battery_service::register_fuel_gauge(dev).await.unwrap();

    spawner.must_spawn(espi_service::task());
}

#[embassy_executor::task]
async fn wrapper_task(wrapper: Wrapper<'static, FuelGaugeController>) {
    loop {
        wrapper.process().await;
        info!("Got new wrapper message");
    }
}

fn main() {
    env_logger::builder().filter_level(log::LevelFilter::Trace).init();

    static EXECUTOR: StaticCell<Executor> = StaticCell::new();
    let executor = EXECUTOR.init(Executor::new());

    let expectations = vec![];

    static DEV: OnceLock<Device> = OnceLock::new();

    let dev = DEV.get_or_init(|| Device::new(DeviceId(0)));

    let wrap = Wrapper::new(
        dev,
        FuelGaugeController {
            driver: MockFuelGaugeDriver::new(Mock::new(&expectations)),
        },
    );
    executor.run(|spawner| {
        spawner.must_spawn(wrapper_task(wrap));
        spawner.must_spawn(battery_service::task());
        spawner.must_spawn(init_task(spawner, dev));
    });
}
