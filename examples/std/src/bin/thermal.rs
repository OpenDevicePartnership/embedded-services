use embassy_executor::{Executor, Spawner};
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::mutex::Mutex;
use embassy_sync::once_lock::OnceLock;
use embassy_time::Timer;
use embedded_fans_async as fan;
use embedded_hal::digital;
use embedded_hal_async::digital as async_digital;
use embedded_sensors_hal_async::sensor;
use embedded_sensors_hal_async::temperature::{DegreesCelsius, TemperatureSensor, TemperatureThresholdSet};
use embedded_services::comms;
use log::info;
use static_cell::StaticCell;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use thermal_service::{default_handler, mptf};

// Mock host service
mod host {
    use embassy_sync::{blocking_mutex::raw::NoopRawMutex, signal::Signal};
    use embedded_services::comms::{self, Endpoint, EndpointID, External, MailboxDelegate};
    use log::{info, warn};
    use thermal_service::{default_handler, mptf};

    pub struct Host {
        pub tp: Endpoint,
        pub alert: Signal<NoopRawMutex, ()>,
    }

    impl Host {
        pub fn new() -> Self {
            Self {
                tp: Endpoint::uninit(EndpointID::External(External::Host)),
                alert: Signal::new(),
            }
        }

        fn handle_response(&self, response: mptf::Response) {
            match response {
                Ok(mptf::ResponseData::GetTmp(tmp)) => {
                    info!("Host received temperature: {} °C", thermal_service::utils::dk_to_c(tmp))
                }
                Ok(mptf::ResponseData::GetVar(uid, value)) => {
                    if uid == default_handler::uid::FAN_CURRENT_RPM {
                        info!("Host received fan RPM: {value}")
                    } else {
                        info!("Received GetVar response: {uid}: {value}")
                    }
                }
                _ => info!("Received MPTF response: {response:?}"),
            }
        }

        fn handle_notify(&self, notification: mptf::Notify) {
            match notification {
                mptf::Notify::Threshold => {
                    warn!("Host received notification that temperature threshold exceeded!");
                    self.alert.signal(());
                }
                _ => info!("Received notification: {notification:?}"),
            }
        }
    }

    impl MailboxDelegate for Host {
        fn receive(&self, message: &comms::Message) -> Result<(), comms::MailboxDelegateError> {
            if let Some(&response) = message.data.get::<mptf::Response>() {
                self.handle_response(response);
                Ok(())
            } else if let Some(&notification) = message.data.get::<mptf::Notify>() {
                self.handle_notify(notification);
                Ok(())
            } else {
                Err(comms::MailboxDelegateError::MessageNotFound)
            }
        }
    }
}

// A mock struct shared by MockSensor and MockAlertPin to sync on raw samples and thresholds
struct MockBus {
    samples: [f32; 35],
    idx: AtomicUsize,
    threshold_low: Mutex<NoopRawMutex, f32>,
    threshold_high: Mutex<NoopRawMutex, f32>,
}

impl MockBus {
    fn new() -> Self {
        Self {
            samples: [
                20.0, 25.0, 30.0, 35.0, 40.0, 45.0, 50.0, 55.0, 60.0, 65.0, 70.0, 75.0, 80.0, 85.0, 90.0, 95.0, 100.0,
                105.0, 100.0, 95.0, 90.0, 85.0, 80.0, 75.0, 70.0, 65.0, 60.0, 55.0, 50.0, 45.0, 40.0, 35.0, 30.0, 25.0,
                20.0,
            ],
            idx: AtomicUsize::new(0),
            threshold_low: Mutex::new(0.0),
            threshold_high: Mutex::new(0.0),
        }
    }

    // Return the current sample
    fn sample(&self) -> f32 {
        self.samples[self.idx.load(Ordering::SeqCst)]
    }

    // Return the current sample and move to next sample (wrapping around at end)
    fn sample_and_next(&self) -> f32 {
        self.samples[self
            .idx
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |idx| {
                Some((idx + 1) % self.samples.len())
            })
            .unwrap()]
    }

    async fn threshold_high(&self) -> f32 {
        *self.threshold_high.lock().await
    }

    async fn set_threshold_low(&self, threshold: f32) {
        *self.threshold_low.lock().await = threshold
    }

    async fn set_threshold_high(&self, threshold: f32) {
        *self.threshold_high.lock().await = threshold
    }
}

#[derive(Copy, Clone, Debug)]
struct MockSensorError;
impl sensor::Error for MockSensorError {
    fn kind(&self) -> sensor::ErrorKind {
        sensor::ErrorKind::Other
    }
}

// A mock temperature sensor
struct MockSensor {
    bus: &'static MockBus,
}

impl MockSensor {
    fn new(bus: &'static MockBus) -> Self {
        Self { bus }
    }
}

impl sensor::ErrorType for MockSensor {
    type Error = MockSensorError;
}

impl TemperatureSensor for MockSensor {
    async fn temperature(&mut self) -> Result<DegreesCelsius, Self::Error> {
        Ok(self.bus.sample_and_next())
    }
}

impl TemperatureThresholdSet for MockSensor {
    async fn set_temperature_threshold_low(&mut self, threshold: DegreesCelsius) -> Result<(), Self::Error> {
        self.bus.set_threshold_low(threshold).await;
        Ok(())
    }

    async fn set_temperature_threshold_high(&mut self, threshold: DegreesCelsius) -> Result<(), Self::Error> {
        self.bus.set_threshold_high(threshold).await;
        Ok(())
    }
}

#[derive(Copy, Clone, Debug)]
struct MockFanError;
impl fan::Error for MockFanError {
    fn kind(&self) -> embedded_fans_async::ErrorKind {
        fan::ErrorKind::Other
    }
}

// A mock fan
struct MockFan {
    rpm: u16,
}

impl MockFan {
    fn new() -> Self {
        Self { rpm: 0 }
    }
}

impl fan::ErrorType for MockFan {
    type Error = MockFanError;
}

impl fan::Fan for MockFan {
    fn min_rpm(&self) -> u16 {
        0
    }

    fn max_rpm(&self) -> u16 {
        5000
    }

    fn min_start_rpm(&self) -> u16 {
        1000
    }

    async fn set_speed_rpm(&mut self, rpm: u16) -> Result<u16, Self::Error> {
        self.rpm = rpm;
        Ok(rpm)
    }
}

impl fan::RpmSense for MockFan {
    async fn rpm(&mut self) -> Result<u16, Self::Error> {
        Ok(self.rpm)
    }
}

#[derive(Copy, Clone, Debug)]
struct MockAlertPinError;
impl digital::Error for MockAlertPinError {
    fn kind(&self) -> digital::ErrorKind {
        digital::ErrorKind::Other
    }
}

struct MockAlertPin {
    bus: &'static MockBus,
}

impl MockAlertPin {
    fn new(bus: &'static MockBus) -> Self {
        Self { bus }
    }
}

impl digital::ErrorType for MockAlertPin {
    type Error = MockAlertPinError;
}

impl async_digital::Wait for MockAlertPin {
    // Periodically wake up then check if the current sample exceeds the set high threshold
    // This is to simulate an interrupt being generated on the GPIO pin by the physical sensor
    async fn wait_for_falling_edge(&mut self) -> Result<(), Self::Error> {
        loop {
            Timer::after_millis(100).await;

            let sample = self.bus.sample();
            let threshold = self.bus.threshold_high().await;

            if sample >= threshold {
                break;
            }
        }

        Ok(())
    }

    // These are unused for this demo
    async fn wait_for_rising_edge(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn wait_for_any_edge(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn wait_for_high(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn wait_for_low(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

// Simulates host receiving requests from OSPM and forwarding to thermal service
#[embassy_executor::task]
async fn host() {
    info!("Spawning host task");

    static HOST: OnceLock<host::Host> = OnceLock::new();
    let host = HOST.get_or_init(host::Host::new);
    info!("Registering host endpoint");
    comms::register_endpoint(host, &host.tp).await.unwrap();

    let thermal_id = comms::EndpointID::Internal(comms::Internal::Thermal);

    // Set thresholds to 100 °C (3731 deciKelvin)
    host.tp
        .send(thermal_id, &mptf::Request::SetThrs(0, 0, 0, 3731))
        .await
        .unwrap();
    Timer::after_millis(100).await;

    // Set Fan ON temp to 40 °C (3131 deciKelvin)
    host.tp
        .send(
            thermal_id,
            &mptf::Request::SetVar(default_handler::uid::FAN_ON_TEMP, 3131),
        )
        .await
        .unwrap();
    Timer::after_millis(100).await;

    // Set Fan RAMP temp to 50 °C (3231 deciKelvin)
    host.tp
        .send(
            thermal_id,
            &mptf::Request::SetVar(default_handler::uid::FAN_RAMP_TEMP, 3231),
        )
        .await
        .unwrap();
    Timer::after_millis(100).await;

    // Set Fan MAX temp to 80 °C (3531 deciKelvin)
    host.tp
        .send(
            thermal_id,
            &mptf::Request::SetVar(default_handler::uid::FAN_MAX_TEMP, 3531),
        )
        .await
        .unwrap();
    Timer::after_millis(100).await;

    // Wait to receive MPTF notification that threshold exceeded, then request temperature and RPM
    loop {
        host.alert.wait().await;

        info!("Host requesting temperature in response to threshold alert");
        host.tp.send(thermal_id, &mptf::Request::GetTmp(0)).await.unwrap();

        // Need to wait briefly before send is fixed to propagate errors and we can handle retries
        Timer::after_millis(100).await;

        info!("Host requesting fan RPM in response to threshold alert");
        host.tp
            .send(
                thermal_id,
                &mptf::Request::GetVar(default_handler::uid::FAN_CURRENT_RPM),
            )
            .await
            .unwrap();
    }
}

async fn init_sensor(spawner: Spawner) {
    info!("Initializing mock bus");
    static BUS: OnceLock<MockBus> = OnceLock::new();
    let bus = BUS.get_or_init(MockBus::new);

    info!("Initializing alert pin");
    let mock_pin = MockAlertPin::new(bus);

    info!("Initializing mock sensor");
    let mock_sensor = MockSensor::new(bus);
    static SENSOR: OnceLock<thermal_service::sensor::Sensor<MockSensor>> = OnceLock::new();
    let sensor = SENSOR.get_or_init(|| thermal_service::sensor::Sensor::new(default_handler::SENSOR, mock_sensor));

    thermal_service::register_sensor(sensor.device()).await.unwrap();
    spawner.must_spawn(mock_sensor_task(sensor, mock_pin));

    thermal_service::execute_sensor_request(
        default_handler::SENSOR,
        thermal_service::sensor::Request::SetSamplingPeriod(1000),
    )
    .await
    .unwrap();
}

async fn init_fan(spawner: Spawner) {
    info!("Initializing mock fan");
    let mock_fan = MockFan::new();
    static FAN: OnceLock<thermal_service::fan::Fan<MockFan>> = OnceLock::new();
    let fan = FAN.get_or_init(|| thermal_service::fan::Fan::new(default_handler::FAN, mock_fan));

    thermal_service::register_fan(fan.device()).await.unwrap();
    spawner.must_spawn(mock_fan_task(fan));
}

async fn init_thermal(spawner: Spawner) {
    info!("Initializing thermal service");

    // Initialize the default MPTF handler, then initialize the thermal service with it
    static HANDLER: StaticCell<thermal_service::default_handler::DefaultHandler> = StaticCell::new();
    let handler = HANDLER.init(thermal_service::default_handler::DefaultHandler::new(
        thermal_service::default_handler::Config::default(),
    ));
    thermal_service::init(spawner, handler).await;

    init_sensor(spawner).await;
    init_fan(spawner).await;

    // TODO: Ideally move this into library, but depends on sensor and fan being registered first
    handler.init().await.unwrap();
    spawner.must_spawn(default_handler::task(spawner, handler));
}

#[embassy_executor::task]
async fn run(spawner: Spawner) {
    embedded_services::init().await;
    init_thermal(spawner).await;
    spawner.must_spawn(host());
}

fn main() {
    env_logger::builder().filter_level(log::LevelFilter::Info).init();

    static EXECUTOR: StaticCell<Executor> = StaticCell::new();
    let executor = EXECUTOR.init(Executor::new());
    executor.run(|spawner| {
        spawner.must_spawn(run(spawner));
    });
}

thermal_service::impl_sensor_task!(mock_sensor_task, MockSensor, MockAlertPin);
thermal_service::impl_fan_task!(mock_fan_task, MockFan);
