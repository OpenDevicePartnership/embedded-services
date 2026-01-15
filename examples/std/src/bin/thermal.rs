use embassy_executor::{Executor, Spawner};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::mutex::Mutex;
use embassy_sync::once_lock::OnceLock;
use embassy_time::Timer;
use embedded_fans_async as fan;
use embedded_sensors_hal_async::sensor;
use embedded_sensors_hal_async::temperature::{DegreesCelsius, TemperatureSensor, TemperatureThresholdSet};
use embedded_services::comms;
use log::{info, warn};
use static_cell::StaticCell;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use thermal_service as ts;
use thermal_service_messages::{
    ThermalGetTmpRequest, ThermalGetVarRequest, ThermalRequest, ThermalSetThrsRequest, ThermalSetVarRequest,
};
use ts::mptf;

// Mock host service
mod host {
    use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};
    use embedded_services::comms::{self, Endpoint, EndpointID, External, MailboxDelegate};
    use log::{info, warn};
    use thermal_service as ts;
    use thermal_service_messages::{ThermalGetTmpResponse, ThermalGetVarResponse, ThermalResponse, ThermalResult};
    use ts::mptf;

    pub struct Host {
        pub tp: Endpoint,
        pub alert: Signal<CriticalSectionRawMutex, ()>,
    }

    impl Host {
        pub fn new() -> Self {
            Self {
                tp: Endpoint::uninit(EndpointID::External(External::Host)),
                alert: Signal::new(),
            }
        }

        fn handle_response(&self, response: ThermalResponse) {
            match response {
                ThermalResponse::ThermalGetTmpResponse(ThermalGetTmpResponse { temperature }) => {
                    info!("Host received temperature: {} °C", ts::utils::dk_to_c(temperature))
                }
                ThermalResponse::ThermalGetVarResponse(ThermalGetVarResponse { val }) => {
                    info!("Host received fan RPM: {val}")
                }
                _ => info!("Received MPTF response: {response:?}"),
            }
        }
    }

    impl MailboxDelegate for Host {
        fn receive(&self, message: &comms::Message) -> Result<(), comms::MailboxDelegateError> {
            if let Some(&result) = message.data.get::<ThermalResult>() {
                self.handle_response(result.map_err(|_| comms::MailboxDelegateError::Other)?);
                Ok(())
            } else if let Some(&notification) = message.data.get::<mptf::Notify>() {
                warn!("Received notification: {notification:?}");
                self.alert.signal(());
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
    threshold_low: Mutex<CriticalSectionRawMutex, f32>,
    threshold_high: Mutex<CriticalSectionRawMutex, f32>,
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

    // Return the current sample and move to next sample (wrapping around at end)
    fn sample_and_next(&self) -> f32 {
        self.samples[self
            .idx
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |idx| {
                Some((idx + 1) % self.samples.len())
            })
            .unwrap()]
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

impl ts::sensor::CustomRequestHandler for MockSensor {}
impl ts::sensor::Controller for MockSensor {}

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
        1000
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

impl ts::fan::CustomRequestHandler for MockFan {}
impl ts::fan::RampResponseHandler for MockFan {}
impl ts::fan::Controller for MockFan {}

// Simulates host receiving requests from OSPM and forwarding to thermal service
#[embassy_executor::task]
async fn host() {
    info!("Spawning host task");

    static HOST: OnceLock<host::Host> = OnceLock::new();
    let host = HOST.get_or_init(host::Host::new);
    info!("Registering host endpoint");
    comms::register_endpoint(host, &host.tp).await.unwrap();

    let thermal_id = comms::EndpointID::Internal(comms::Internal::Thermal);

    // Set thresholds to 40 °C (3131 deciKelvin)
    host.tp
        .send(
            thermal_id,
            &ThermalRequest::ThermalSetThrsRequest(ThermalSetThrsRequest {
                instance_id: 0,
                timeout: 0.into(),
                low: 0.into(),
                high: 3131.into(),
            }),
        )
        .await
        .unwrap();
    Timer::after_millis(100).await;

    // Set Fan ON temp to 40 °C (3131 deciKelvin)
    host.tp
        .send(
            thermal_id,
            &ThermalRequest::ThermalSetVarRequest(ThermalSetVarRequest {
                instance_id: 0,
                len: 4.into(),
                var_uuid: mptf::uuid_standard::FAN_ON_TEMP,
                set_var: 3131.into(),
            }),
        )
        .await
        .unwrap();
    Timer::after_millis(100).await;

    // Set Fan RAMP temp to 50 °C (3231 deciKelvin)
    host.tp
        .send(
            thermal_id,
            &ThermalRequest::ThermalSetVarRequest(ThermalSetVarRequest {
                instance_id: 0,
                len: 4.into(),
                var_uuid: mptf::uuid_standard::FAN_RAMP_TEMP,
                set_var: 3231.into(),
            }),
        )
        .await
        .unwrap();
    Timer::after_millis(100).await;

    // Set Fan MAX temp to 80 °C (3531 deciKelvin)
    host.tp
        .send(
            thermal_id,
            &ThermalRequest::ThermalSetVarRequest(ThermalSetVarRequest {
                instance_id: 0,
                len: 4.into(),
                var_uuid: mptf::uuid_standard::FAN_MAX_TEMP,
                set_var: 3531.into(),
            }),
        )
        .await
        .unwrap();
    Timer::after_millis(100).await;

    // Wait to receive MPTF notification that threshold exceeded, then request temperature and RPM
    loop {
        host.alert.wait().await;

        info!("Host requesting temperature in response to threshold alert");
        host.tp
            .send(
                thermal_id,
                &ThermalRequest::ThermalGetTmpRequest(ThermalGetTmpRequest { instance_id: 0 }),
            )
            .await
            .unwrap();

        // Need to wait briefly before send is fixed to propagate errors and we can handle retries
        Timer::after_millis(100).await;

        info!("Host requesting fan RPM in response to threshold alert");
        host.tp
            .send(
                thermal_id,
                &ThermalRequest::ThermalGetVarRequest(ThermalGetVarRequest {
                    instance_id: 0,
                    len: 4.into(),
                    var_uuid: mptf::uuid_standard::FAN_CURRENT_RPM,
                }),
            )
            .await
            .unwrap();
    }
}

async fn init_sensor(spawner: Spawner) {
    info!("Initializing mock bus");
    static BUS: OnceLock<MockBus> = OnceLock::new();
    let bus = BUS.get_or_init(MockBus::new);

    info!("Initializing mock sensor");
    let mock_sensor = MockSensor::new(bus);
    static SENSOR: OnceLock<ts::sensor::Sensor<MockSensor, 16>> = OnceLock::new();

    let profile = ts::sensor::Profile {
        warn_high_threshold: 40.0,
        prochot_threshold: 50.0,
        crt_threshold: 80.0,
        ..Default::default()
    };
    let sensor = SENSOR.get_or_init(|| ts::sensor::Sensor::new(ts::sensor::DeviceId(0), mock_sensor, profile));

    ts::register_sensor(sensor.device()).await.unwrap();
    spawner.must_spawn(mock_sensor_task(sensor));
}

async fn init_fan(spawner: Spawner) {
    info!("Initializing mock fan");
    let mock_fan = MockFan::new();
    static FAN: OnceLock<ts::fan::Fan<MockFan, 16>> = OnceLock::new();
    let fan = FAN.get_or_init(|| ts::fan::Fan::new(ts::fan::DeviceId(0), mock_fan, ts::fan::Profile::default()));

    ts::register_fan(fan.device()).await.unwrap();
    spawner.must_spawn(mock_fan_task(fan));
}

async fn init_thermal(spawner: Spawner) {
    info!("Initializing thermal service");
    ts::init().await.unwrap();

    init_sensor(spawner).await;
    init_fan(spawner).await;
}

#[embassy_executor::task]
async fn handle_alerts() {
    loop {
        match ts::wait_event().await {
            ts::Event::ThresholdExceeded(ts::sensor::DeviceId(sensor_id), ts::sensor::ThresholdType::WarnHigh, _) => {
                warn!("Sensor {sensor_id} exceeded WARN threshold");
                ts::send_service_msg(comms::EndpointID::External(comms::External::Host), &mptf::Notify::Warn)
                    .await
                    .unwrap()
            }
            ts::Event::ThresholdExceeded(ts::sensor::DeviceId(sensor_id), ts::sensor::ThresholdType::Prochot, _) => {
                warn!("Sensor {sensor_id} exceeded PROCHOT threshold");
                ts::send_service_msg(
                    comms::EndpointID::External(comms::External::Host),
                    &mptf::Notify::ProcHot,
                )
                .await
                .unwrap()
            }
            ts::Event::ThresholdExceeded(ts::sensor::DeviceId(sensor_id), ts::sensor::ThresholdType::Critical, _) => {
                warn!("Sensor {sensor_id} exceeded CRITICAL threshold");
                ts::send_service_msg(
                    comms::EndpointID::External(comms::External::Host),
                    &mptf::Notify::Critical,
                )
                .await
                .unwrap()
            }
            event => warn!("Event: {event:?}"),
        }
    }
}

#[embassy_executor::task]
async fn handle_requests() -> ! {
    ts::task::handle_requests().await;
    unreachable!()
}

#[embassy_executor::task]
async fn run(spawner: Spawner) {
    embedded_services::init().await;
    init_thermal(spawner).await;
    spawner.must_spawn(host());
    spawner.must_spawn(handle_alerts());
    spawner.must_spawn(handle_requests());
}

fn main() {
    env_logger::builder().filter_level(log::LevelFilter::Trace).init();

    static EXECUTOR: StaticCell<Executor> = StaticCell::new();
    let executor = EXECUTOR.init(Executor::new());
    executor.run(|spawner| {
        spawner.must_spawn(run(spawner));
    });
}

#[embassy_executor::task]
async fn mock_sensor_task(sensor: &'static ts::sensor::Sensor<MockSensor, 16>) -> ! {
    ts::task::sensor_task(sensor).await;
    unreachable!()
}

#[embassy_executor::task]
async fn mock_fan_task(fan: &'static ts::fan::Fan<MockFan, 16>) -> ! {
    ts::task::fan_task(fan).await;
    unreachable!()
}
