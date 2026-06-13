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
use bs::FuelGauge as _;
use embassy_sync::mutex::Mutex;
use embedded_batteries_async::smart_battery::{BatteryModeFields, SmartBattery};
use embedded_services::GlobalRawMutex;

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

impl From<BatteryError> for bs::FuelGaugeError {
    fn from(_value: BatteryError) -> Self {
        bs::FuelGaugeError::BusError
    }
}

/// Platform specific fuel gauge. Owns its own state, as managed via the [`bs::FuelGauge`] trait.
struct Battery {
    pub driver: Bq40z50R5<pico_de_gallo_hal::I2c, pico_de_gallo_hal::Delay>,
    state: bs::State,
}

embedded_batteries_async::impl_smart_battery_for_wrapper_type!(Battery, driver, BatteryError);

impl bs::FuelGauge for Battery {
    type FuelGaugeError = BatteryError;

    async fn initialize(&mut self) -> Result<(), Self::FuelGaugeError> {
        self.driver
            // Milliamps
            .set_battery_mode(BatteryModeFields::with_capacity_mode(BatteryModeFields::new(), false))
            .await
            .inspect_err(|_| embedded_services::error!("FG: failed to initialize"))?;

        embedded_services::info!("FG: initialized");
        self.state_mut().on_initialized();
        Ok(())
    }

    async fn ping(&mut self) -> Result<(), Self::FuelGaugeError> {
        if let Err(e) = self.driver.voltage().await {
            embedded_services::error!("FG: failed to ping");
            Err(e.into())
        } else {
            embedded_services::info!("FG: ping success");
            self.state_mut().on_recovered();
            Ok(())
        }
    }

    async fn update_static_data(&mut self) -> Result<(), Self::FuelGaugeError> {
        let mut buf = [0u8; 21];
        let mut new_msgs = bs::StaticBatteryMsgs {
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

        self.state_mut().on_static_data(new_msgs);
        Ok(())
    }

    async fn update_dynamic_data(&mut self) -> Result<(), Self::FuelGaugeError> {
        let new_msgs = bs::DynamicBatteryMsgs {
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
        self.state_mut().on_dynamic_data(new_msgs);
        Ok(())
    }

    fn state(&self) -> &bs::State {
        &self.state
    }

    fn state_mut(&mut self) -> &mut bs::State {
        &mut self.state
    }
}

/// The fuel gauge, wrapped in a mutex so it can be shared between the OEM driving
/// code and the battery service.
type FuelGauge = Mutex<GlobalRawMutex, Battery>;
/// The registration: a single fuel gauge, which becomes battery `0`.
type Reg<'hw> = bs::ArrayRegistration<'hw, FuelGauge, 1>;

async fn init_state_machine(fuel_gauge: &FuelGauge) -> Result<(), BatteryError> {
    let mut fg = fuel_gauge.lock().await;
    fg.initialize()
        .await
        .inspect_err(|f| embedded_services::debug!("Fuel gauge init error: {:?}", f))?;
    fg.update_static_data()
        .await
        .inspect_err(|f| embedded_services::debug!("Fuel gauge static data error: {:?}", f))?;
    fg.update_dynamic_data()
        .await
        .inspect_err(|f| embedded_services::debug!("Fuel gauge dynamic data error: {:?}", f))?;
    Ok(())
}

async fn recover_state_machine(fuel_gauge: &FuelGauge) -> Result<(), ()> {
    let mut retries = 5u32;
    loop {
        let result = fuel_gauge.lock().await.ping().await;
        if result.is_ok() {
            embedded_services::info!("FG recovered!");
            return Ok(());
        }
        retries = retries.saturating_sub(1);
        if retries == 0 {
            embedded_services::error!("Couldn't recover, reinit needed");
            return Err(());
        }
        embedded_services::trace!("Recovery failed, trying again after a backoff period");
        tokio::time::sleep(std::time::Duration::from_secs(10)).await;
    }
}

async fn run_app<'hw>(battery_service: bs::Service<'hw, Reg<'hw>>) {
    // Initialize the fuel gauge by driving it directly.
    let fuel_gauge = battery_service.registration.fuel_gauges[0];
    let mut retries = 5;
    while let Err(e) = init_state_machine(fuel_gauge).await {
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
        if count.is_multiple_of(const { 60 * 60 * 60 })
            && let Err(e) = fuel_gauge.lock().await.update_static_data().await
        {
            failures += 1;
            embedded_services::error!("Fuel gauge static data error: {:#?}", e);
        }
        if let Err(e) = fuel_gauge.lock().await.update_dynamic_data().await {
            failures += 1;
            embedded_services::error!("Fuel gauge dynamic data error: {:#?}", e);
        }

        // The battery service answers ACPI queries by reading the fuel gauge's
        // cached state. The caller hands the service exclusive access to the
        // fuel gauge for the duration of the query.
        {
            let mut fg = fuel_gauge.lock().await;
            if battery_service.battery_status(&mut *fg).is_ok() {
                embedded_services::debug!("Queried battery status via the battery service");
            }
        }

        if failures > 10 {
            failures = 0;
            count = 0;
            embedded_services::error!("FG: Too many errors, timing out and starting recovery...");
            if recover_state_machine(fuel_gauge).await.is_err() {
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

    embedded_services::debug!("Initializing battery service");
    embedded_services::init().await;

    let p = pico_de_gallo_hal::Hal::new();

    // The OEM owns the fuel gauge. A shared reference is handed both to the
    // service (via registration) and to the code that drives it.
    let fuel_gauge: FuelGauge = Mutex::new(Battery {
        driver: Bq40z50R5::new(p.i2c(), p.delay()),
        state: bs::State::default(),
    });

    let battery_service = bs::Service::new(bs::ArrayRegistration {
        fuel_gauges: [&fuel_gauge],
    });

    run_app(battery_service).await;
}
