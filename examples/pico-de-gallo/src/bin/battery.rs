//! Pico-de-gallo battery example
//!
//! Runs the ODP battery service in a std environment, using the [pico-de-gallo](https://github.com/OpenDevicePartnership/pico-de-gallo)
//! as a sensor bridge to a [Texas Instruments BQ40Z50-R5 battery fuel gauge EVK](https://github.com/OpenDevicePartnership/bq40z50).
//!
//! The hardware setup should be as follows:
//!
//!      ___________            ___________             ____________
//!     |           |          |   pico-   | <--SDA--> | BQ40Z50-R5 |
//!     |  Host PC  | <-USB--> |  de-gallo | <--SCL--> | Fuel Gauge |
//!     |___________|          |___________| <--GND--> |____________|
//!
//! The host PC running the battery-service should be connected via USB to the pico-de-gallo. The BQ40Z50-R5 EVK should be connected
//! to the pico-de-gallo's I2C lines (don't forget GND!). The BQ40Z50-R5 EVK should be connected to the appropriate power supply and
//! battery cells, as outlined in its datasheet.
//!
//! The example can be run simply by typing `cargo run --bin battery`

use battery_service as bs;
use bq40z50_rx::{BQ40Z50Error, Bq40z50R5};
use embedded_batteries_async::smart_battery::{BatteryModeFields, SmartBattery};
use static_cell::StaticCell;

/// Platform specific battery errors.
#[derive(Debug)]
enum BatteryError {
    /// Generic failure
    Failed,
}

impl embedded_batteries_async::smart_battery::Error for BatteryError {
    fn kind(&self) -> embedded_batteries_async::smart_battery::ErrorKind {
        embedded_batteries_async::smart_battery::ErrorKind::Other
    }
}

impl From<BQ40Z50Error<pico_de_gallo_hal::Error>> for BatteryError {
    fn from(_value: BQ40Z50Error<pico_de_gallo_hal::Error>) -> Self {
        BatteryError::Failed
    }
}

/// Platform specific battery controller.
struct Battery {
    pub driver: Bq40z50R5<pico_de_gallo_hal::I2c, pico_de_gallo_hal::Delay>,
}

embedded_batteries_async::impl_smart_battery_for_wrapper_type!(Battery, driver, BatteryError);

impl bs::controller::Controller for Battery {
    type ControllerError = BatteryError;

    async fn initialize(&mut self) -> Result<(), Self::ControllerError> {
        self.driver
            // Milliamps
            .set_battery_mode(BatteryModeFields::with_capacity_mode(BatteryModeFields::new(), false))
            .await
            .inspect_err(|_| embedded_services::error!("FG: failed to initialize"))?;

        embedded_services::info!("FG: initialized");
        Ok(())
    }

    async fn get_static_data(&mut self) -> Result<bs::device::StaticBatteryMsgs, Self::ControllerError> {
        let mut buf = [0u8; 21];
        let mut new_msgs = bs::device::StaticBatteryMsgs {
            design_capacity_mwh: match self.design_capacity().await? {
                embedded_batteries_async::smart_battery::CapacityModeValue::CentiWattUnsigned(_) => 0xDEADBEEF,
                embedded_batteries_async::smart_battery::CapacityModeValue::MilliAmpUnsigned(design_capacity) => {
                    design_capacity.into()
                }
            },
            design_voltage_mv: self.design_voltage().await?,
            ..Default::default()
        };

        let buf_len = new_msgs.device_chemistry.len();
        self.device_chemistry(&mut buf[..buf_len]).await?;
        new_msgs.device_chemistry.copy_from_slice(&buf[..buf_len]);

        Ok(new_msgs)
    }

    async fn get_dynamic_data(&mut self) -> Result<bs::device::DynamicBatteryMsgs, Self::ControllerError> {
        let new_msgs = bs::device::DynamicBatteryMsgs {
            average_current_ma: self.average_current().await?,
            battery_status: self.battery_status().await?.into(),
            max_power_mw: self
                .driver
                .device
                .max_turbo_power()
                .read_async()
                .await?
                .max_turbo_power()
                .unsigned_abs()
                .into(),
            battery_temp_dk: self.temperature().await?,
            sus_power_mw: self
                .driver
                .device
                .sus_turbo_power()
                .read_async()
                .await?
                .sus_turbo_power()
                .unsigned_abs()
                .into(),
            charging_current_ma: self.charging_current().await?,
            charging_voltage_mv: self.charging_voltage().await?,
            voltage_mv: self.voltage().await?,
            current_ma: self.current().await?,
            full_charge_capacity_mwh: match self.full_charge_capacity().await? {
                embedded_batteries_async::smart_battery::CapacityModeValue::CentiWattUnsigned(_) => 0xDEADBEEF,
                embedded_batteries_async::smart_battery::CapacityModeValue::MilliAmpUnsigned(capacity) => {
                    capacity.into()
                }
            },
            remaining_capacity_mwh: match self.remaining_capacity().await? {
                embedded_batteries_async::smart_battery::CapacityModeValue::CentiWattUnsigned(_) => 0xDEADBEEF,
                embedded_batteries_async::smart_battery::CapacityModeValue::MilliAmpUnsigned(capacity) => {
                    capacity.into()
                }
            },
            relative_soc_pct: self.relative_state_of_charge().await?.into(),
            cycle_count: self.cycle_count().await?,
            max_error_pct: self.max_error().await?.into(),
            bmd_status: embedded_batteries_async::acpi::BmdStatusFlags::default(),
            turbo_vload_mv: 0,
            turbo_rhf_effective_mohm: 0,
        };
        Ok(new_msgs)
    }

    async fn get_device_event(&mut self) -> bs::controller::ControllerEvent {
        loop {
            tokio::task::yield_now().await;
        }
    }

    async fn ping(&mut self) -> Result<(), Self::ControllerError> {
        if let Err(e) = self.driver.voltage().await {
            embedded_services::error!("FG: failed to ping");
            Err(e.into())
        } else {
            embedded_services::info!("FG: ping success");
            Ok(())
        }
    }

    fn set_timeout(&mut self, _duration: embassy_time::Duration) {
        embassy_time::Duration::from_secs(60);
    }
}

async fn init_and_run_service(
    battery_service: &'static battery_service::Service,
    i2c: pico_de_gallo_hal::I2c,
    delay: pico_de_gallo_hal::Delay,
) -> ! {
    embedded_services::debug!("Initializing battery service");
    embedded_services::init().await;

    static BATTERY_DEVICE: StaticCell<bs::device::Device> = StaticCell::new();
    static BATTERY_WRAPPER: StaticCell<bs::wrapper::Wrapper<'static, Battery>> = StaticCell::new();
    let device = BATTERY_DEVICE.init(bs::device::Device::new(bs::device::DeviceId(0)));

    let wrapper = BATTERY_WRAPPER.init(bs::wrapper::Wrapper::new(
        device,
        Battery {
            driver: Bq40z50R5::new(i2c, delay),
        },
    ));

    // Run battery service
    let _ = embassy_futures::join::join(
        tokio::spawn(battery_service::task::task(battery_service, [device])),
        tokio::spawn(wrapper.process()),
    )
    .await;
    unreachable!()
}

async fn init_state_machine(battery_service: &'static bs::Service) -> Result<(), bs::context::ContextError> {
    battery_service
        .execute_event(battery_service::context::BatteryEvent {
            event: battery_service::context::BatteryEventInner::DoInit,
            device_id: bs::device::DeviceId(0),
        })
        .await
        .inspect_err(|f| embedded_services::debug!("Fuel gauge init error: {:?}", f))?;

    battery_service
        .execute_event(battery_service::context::BatteryEvent {
            event: battery_service::context::BatteryEventInner::PollStaticData,
            device_id: bs::device::DeviceId(0),
        })
        .await
        .inspect_err(|f| embedded_services::debug!("Fuel gauge static data error: {:?}", f))?;

    battery_service
        .execute_event(battery_service::context::BatteryEvent {
            event: battery_service::context::BatteryEventInner::PollDynamicData,
            device_id: bs::device::DeviceId(0),
        })
        .await
        .inspect_err(|f| embedded_services::debug!("Fuel gauge dynamic data error: {:?}", f))?;

    Ok(())
}

async fn recover_state_machine(battery_service: &'static battery_service::Service) -> Result<(), ()> {
    loop {
        match battery_service
            .execute_event(battery_service::context::BatteryEvent {
                event: battery_service::context::BatteryEventInner::Timeout,
                device_id: bs::device::DeviceId(0),
            })
            .await
        {
            Ok(_) => {
                embedded_services::info!("FG recovered!");
                return Ok(());
            }
            Err(e) => match e {
                battery_service::context::ContextError::StateError(e) => match e {
                    battery_service::context::StateMachineError::DeviceTimeout => {
                        embedded_services::trace!("Recovery failed, trying again after a backoff period");
                        tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                    }
                    battery_service::context::StateMachineError::NoOpRecoveryFailed => {
                        embedded_services::error!("Couldn't recover, reinit needed");
                        return Err(());
                    }
                    _ => embedded_services::debug!("Unexpected error"),
                },
                _ => embedded_services::debug!("Unexpected error"),
            },
        }
    }
}

pub async fn run_app(battery_service: &'static battery_service::Service) {
    // Initialize battery state machine.
    let mut retries = 5;
    while let Err(e) = init_state_machine(battery_service).await {
        retries -= 1;
        if retries <= 0 {
            embedded_services::error!("Failed to initialize Battery: {:?}", e);
            return;
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }

    let mut failures: u32 = 0;
    let mut count: usize = 1;
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        if count.is_multiple_of(const { 60 * 60 * 60 }) {
            if let Err(e) = battery_service
                .execute_event(battery_service::context::BatteryEvent {
                    event: battery_service::context::BatteryEventInner::PollStaticData,
                    device_id: bs::device::DeviceId(0),
                })
                .await
            {
                failures += 1;
                embedded_services::error!("Fuel gauge static data error: {:#?}", e);
            }
        }
        if let Err(e) = battery_service
            .execute_event(battery_service::context::BatteryEvent {
                event: battery_service::context::BatteryEventInner::PollDynamicData,
                device_id: bs::device::DeviceId(0),
            })
            .await
        {
            failures += 1;
            embedded_services::error!("Fuel gauge dynamic data error: {:#?}", e);
        }

        if failures > 10 {
            failures = 0;
            count = 0;
            embedded_services::error!("FG: Too many errors, timing out and starting recovery...");
            if recover_state_machine(battery_service).await.is_err() {
                embedded_services::error!("FG: Fatal error");
                return;
            }
        }

        count = count.wrapping_add(1);
    }
}

#[tokio::main]
async fn main() {
    env_logger::builder().filter_level(log::LevelFilter::Info).init();
    embedded_services::info!("host: battery example started");

    static BATTERY_SERVICE: bs::Service = bs::Service::new();

    let p = pico_de_gallo_hal::Hal::new();

    let _ = embassy_futures::join::join(
        tokio::spawn(run_app(&BATTERY_SERVICE)),
        tokio::spawn(init_and_run_service(&BATTERY_SERVICE, p.i2c(), p.delay())),
    )
    .await;
}
