use crate::utils::SampleBuf;
use core::marker::PhantomData;
use embassy_sync::mutex::Mutex;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Timer};
use embedded_fans_async::Error as _;
use embedded_sensors_hal_async::temperature::DegreesCelsius;
use embedded_services::event::Sender;
use embedded_services::{GlobalRawMutex, error, trace};
use thermal_service_interface::{fan, sensor};

/// Fan service configuration parameters.
#[derive(Clone, Copy, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Config {
    /// Rate at which to sample the fan RPM.
    pub sample_period: Duration,
    /// Rate at which to update the fan state based on temperature readings when auto control is enabled.
    pub update_period: Duration,
    /// Whether automatic fan control based on temperature is enabled.
    pub auto_control: bool,
    /// Hysteresis value to prevent rapid toggling between fan states when temperature is around a state transition point.
    pub hysteresis: DegreesCelsius,
    /// Temperature at which the fan will turn on and begin running at its minimum RPM.
    pub min_temp: DegreesCelsius,
    /// Temperature at which the fan will follow a speed curve between its minimum and maximum RPM.
    pub ramp_temp: DegreesCelsius,
    /// Temperature at which the fan will run at its maximum RPM.
    pub max_temp: DegreesCelsius,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            sample_period: Duration::from_secs(1),
            update_period: Duration::from_secs(1),
            auto_control: true,
            hysteresis: 2.0,
            min_temp: 25.0,
            ramp_temp: 35.0,
            max_temp: 45.0,
        }
    }
}

struct ServiceInner<T: fan::Driver, const SAMPLE_BUF_LEN: usize> {
    driver: Mutex<GlobalRawMutex, T>,
    state: Mutex<GlobalRawMutex, fan::State>,
    en_signal: Signal<GlobalRawMutex, ()>,
    config: Mutex<GlobalRawMutex, Config>,
    samples: Mutex<GlobalRawMutex, SampleBuf<u16, SAMPLE_BUF_LEN>>,
}

impl<T: fan::Driver, const SAMPLE_BUF_LEN: usize> ServiceInner<T, SAMPLE_BUF_LEN> {
    fn new(driver: T, config: Config) -> Self {
        Self {
            driver: Mutex::new(driver),
            state: Mutex::new(fan::State::Off),
            en_signal: Signal::new(),
            config: Mutex::new(config),
            samples: Mutex::new(SampleBuf::create()),
        }
    }

    async fn handle_sampling(&self) {
        loop {
            match self.driver.lock().await.rpm().await {
                Ok(rpm) => self.samples.lock().await.push(rpm),
                Err(e) => error!("Fan error sampling fan rpm: {:?}", e.kind()),
            }

            let period = self.config.lock().await.sample_period;
            Timer::after(period).await;
        }
    }

    async fn change_state(&self, to: fan::State) -> Result<(), fan::Error> {
        let mut driver = self.driver.lock().await;
        match to {
            fan::State::Off => {
                driver.stop().await.map_err(|_| fan::Error::Hardware)?;
            }
            fan::State::On(fan::OnState::Min) => {
                driver.start().await.map_err(|_| fan::Error::Hardware)?;
            }
            fan::State::On(fan::OnState::Ramping) => {
                // Ramp state will continuously update RPM according to its ramp response function
            }
            fan::State::On(fan::OnState::Max) => {
                let max_rpm = driver.max_rpm();
                let _ = driver.set_speed_rpm(max_rpm).await.map_err(|_| fan::Error::Hardware)?;
            }
        }
        drop(driver);

        let mut state = self.state.lock().await;
        trace!("Fan transitioned to {:?} state from {:?} state", to, *state);
        *state = to;

        Ok(())
    }
}

/// Fan service control handle.
pub struct Service<'hw, T: fan::Driver, S: sensor::SensorService, E: Sender<fan::Event>, const SAMPLE_BUF_LEN: usize> {
    inner: &'hw ServiceInner<T, SAMPLE_BUF_LEN>,
    _phantom: PhantomData<(S, E)>,
}

// Note: We can't derive these traits because the compiler thinks our generics then need to be Copy + Clone,
// but we only hold a reference and don't actually need to be that strict
impl<T: fan::Driver, S: sensor::SensorService, E: Sender<fan::Event>, const SAMPLE_BUF_LEN: usize> Clone
    for Service<'_, T, S, E, SAMPLE_BUF_LEN>
{
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: fan::Driver, S: sensor::SensorService, E: Sender<fan::Event>, const SAMPLE_BUF_LEN: usize> Copy
    for Service<'_, T, S, E, SAMPLE_BUF_LEN>
{
}

impl<'hw, T: fan::Driver, S: sensor::SensorService, E: Sender<fan::Event>, const SAMPLE_BUF_LEN: usize> fan::FanService
    for Service<'hw, T, S, E, SAMPLE_BUF_LEN>
{
    async fn enable_auto_control(&self) -> Result<(), fan::Error> {
        self.inner.change_state(fan::State::Off).await?;
        self.inner.config.lock().await.auto_control = true;
        self.inner.en_signal.signal(());
        Ok(())
    }

    async fn rpm(&self) -> u16 {
        self.inner.samples.lock().await.recent()
    }

    async fn min_rpm(&self) -> u16 {
        self.inner.driver.lock().await.min_rpm()
    }

    async fn max_rpm(&self) -> u16 {
        self.inner.driver.lock().await.max_rpm()
    }

    async fn rpm_average(&self) -> u16 {
        self.inner.samples.lock().await.average()
    }

    async fn rpm_immediate(&self) -> Result<u16, fan::Error> {
        self.inner
            .driver
            .lock()
            .await
            .rpm()
            .await
            .map_err(|_| fan::Error::Hardware)
    }

    async fn set_rpm(&self, rpm: u16) -> Result<(), fan::Error> {
        self.inner
            .driver
            .lock()
            .await
            .set_speed_rpm(rpm)
            .await
            .map_err(|_| fan::Error::Hardware)?;
        self.inner.config.lock().await.auto_control = false;
        Ok(())
    }

    async fn set_duty_percent(&self, duty: u8) -> Result<(), fan::Error> {
        self.inner
            .driver
            .lock()
            .await
            .set_speed_percent(duty)
            .await
            .map_err(|_| fan::Error::Hardware)?;
        self.inner.config.lock().await.auto_control = false;
        Ok(())
    }

    async fn stop(&self) -> Result<(), fan::Error> {
        self.inner
            .driver
            .lock()
            .await
            .stop()
            .await
            .map_err(|_| fan::Error::Hardware)?;
        self.inner.config.lock().await.auto_control = false;
        Ok(())
    }

    async fn set_rpm_sampling_period(&self, period: Duration) {
        self.inner.config.lock().await.sample_period = period;
    }

    async fn set_rpm_update_period(&self, period: Duration) {
        self.inner.config.lock().await.update_period = period;
    }

    async fn state_temp(&self, on_state: fan::OnState) -> DegreesCelsius {
        let config = self.inner.config.lock().await;
        match on_state {
            fan::OnState::Min => config.min_temp,
            fan::OnState::Ramping => config.ramp_temp,
            fan::OnState::Max => config.max_temp,
        }
    }

    async fn set_state_temp(&self, on_state: fan::OnState, temp: DegreesCelsius) {
        let mut config = self.inner.config.lock().await;
        match on_state {
            fan::OnState::Min => config.min_temp = temp,
            fan::OnState::Ramping => config.ramp_temp = temp,
            fan::OnState::Max => config.max_temp = temp,
        }
    }
}

/// Parameters required to initialize a fan service.
pub struct InitParams<'hw, T: fan::Driver, S: sensor::SensorService, E: Sender<fan::Event>> {
    /// The underlying fan driver this service will control.
    pub driver: T,
    /// Initial configuration for the fan service.
    pub config: Config,
    /// The sensor service this fan will use to get temperature readings.
    pub sensor_service: S,
    /// Event senders for fan events.
    pub event_senders: &'hw mut [E],
}

/// The memory resources required by the fan.
pub struct Resources<T: fan::Driver, const SAMPLE_BUF_LEN: usize> {
    inner: Option<ServiceInner<T, SAMPLE_BUF_LEN>>,
}

// Note: We can't derive Default unless we trait bound T by Default,
// but we don't want that restriction since the default is just the None case
impl<T: fan::Driver, const SAMPLE_BUF_LEN: usize> Default for Resources<T, SAMPLE_BUF_LEN> {
    fn default() -> Self {
        Self { inner: None }
    }
}

/// A task runner for a fan. Users must run this in an embassy task or similar async execution context.
pub struct Runner<'hw, T: fan::Driver, S: sensor::SensorService, E: Sender<fan::Event>, const SAMPLE_BUF_LEN: usize> {
    service: &'hw ServiceInner<T, SAMPLE_BUF_LEN>,
    sensor: S,
    event_senders: &'hw mut [E],
}

impl<'hw, T: fan::Driver, S: sensor::SensorService, E: Sender<fan::Event>, const SAMPLE_BUF_LEN: usize>
    Runner<'hw, T, S, E, SAMPLE_BUF_LEN>
{
    async fn broadcast_event(&mut self, event: fan::Event) {
        for sender in self.event_senders.iter_mut() {
            sender.send(event).await;
        }
    }

    async fn ramp_response(&self, temp: DegreesCelsius) -> Result<(), fan::Error> {
        let config = *self.service.config.lock().await;

        let mut driver = self.service.driver.lock().await;
        let min_rpm = driver.min_start_rpm();
        let max_rpm = driver.max_rpm();

        // Provide a linear fan response between its min and max RPM relative to temperature between ramp start and max temp
        let rpm = if temp <= config.ramp_temp {
            min_rpm
        } else if temp >= config.max_temp {
            max_rpm
        } else {
            let ratio = (temp - config.ramp_temp) / (config.max_temp - config.ramp_temp);
            let range = (max_rpm - min_rpm) as f32;
            min_rpm + (ratio * range) as u16
        };

        driver
            .set_speed_rpm(rpm)
            .await
            .map(|_| ())
            .map_err(|_| fan::Error::Hardware)
    }

    async fn handle_fan_off_state(&self, temp: DegreesCelsius) -> Result<(), fan::Error> {
        let config = *self.service.config.lock().await;

        if temp >= config.min_temp {
            self.service.change_state(fan::State::On(fan::OnState::Min)).await?;
        }

        Ok(())
    }

    async fn handle_fan_on_state(&self, temp: DegreesCelsius) -> Result<(), fan::Error> {
        let config = *self.service.config.lock().await;

        if temp < (config.min_temp - config.hysteresis) {
            self.service.change_state(fan::State::Off).await?;
        } else if temp >= config.ramp_temp {
            self.service.change_state(fan::State::On(fan::OnState::Ramping)).await?;
        }

        Ok(())
    }

    async fn handle_fan_ramping_state(&self, temp: DegreesCelsius) -> Result<(), fan::Error> {
        let config = *self.service.config.lock().await;

        if temp < (config.ramp_temp - config.hysteresis) {
            self.service.change_state(fan::State::On(fan::OnState::Min)).await?;
        } else if temp >= config.max_temp {
            self.service.change_state(fan::State::On(fan::OnState::Max)).await?;
        } else {
            self.ramp_response(temp).await?;
        }

        Ok(())
    }

    async fn handle_fan_max_state(&self, temp: DegreesCelsius) -> Result<(), fan::Error> {
        let config = *self.service.config.lock().await;

        if temp < (config.max_temp - config.hysteresis) {
            self.service.change_state(fan::State::On(fan::OnState::Ramping)).await?;
        }

        Ok(())
    }

    async fn handle_fan_state(&self, temp: DegreesCelsius) -> Result<(), fan::Error> {
        let state = *self.service.state.lock().await;
        match state {
            fan::State::Off => self.handle_fan_off_state(temp).await,
            fan::State::On(fan::OnState::Min) => self.handle_fan_on_state(temp).await,
            fan::State::On(fan::OnState::Ramping) => self.handle_fan_ramping_state(temp).await,
            fan::State::On(fan::OnState::Max) => self.handle_fan_max_state(temp).await,
        }
    }

    async fn handle_auto_control(&mut self) {
        loop {
            if self.service.config.lock().await.auto_control {
                let temp = self.sensor.temperature().await;
                if let Err(e) = self.handle_fan_state(temp).await {
                    error!("Error handling fan state transition, disabling auto control: {:?}", e);
                    self.service.config.lock().await.auto_control = false;
                    self.broadcast_event(fan::Event::Failure(e)).await;
                }

                let sleep_duration = self.service.config.lock().await.update_period;
                Timer::after(sleep_duration).await;

            // Sleep until auto control is re-enabled
            } else {
                self.service.en_signal.wait().await;
            }
        }
    }
}

impl<'hw, T: fan::Driver, S: sensor::SensorService + 'hw, E: Sender<fan::Event> + 'hw, const SAMPLE_BUF_LEN: usize>
    odp_service_common::runnable_service::ServiceRunner<'hw> for Runner<'hw, T, S, E, SAMPLE_BUF_LEN>
{
    async fn run(mut self) -> embedded_services::Never {
        let service = self.service;
        loop {
            let _ = embassy_futures::join::join(service.handle_sampling(), self.handle_auto_control()).await;
        }
    }
}

impl<'hw, T: fan::Driver, S: sensor::SensorService + 'hw, E: Sender<fan::Event> + 'hw, const SAMPLE_BUF_LEN: usize>
    odp_service_common::runnable_service::Service<'hw> for Service<'hw, T, S, E, SAMPLE_BUF_LEN>
{
    type Runner = Runner<'hw, T, S, E, SAMPLE_BUF_LEN>;
    type Resources = Resources<T, SAMPLE_BUF_LEN>;
    type ErrorType = fan::Error;
    type InitParams = InitParams<'hw, T, S, E>;

    async fn new(
        service_storage: &'hw mut Self::Resources,
        init_params: Self::InitParams,
    ) -> Result<(Self, Self::Runner), Self::ErrorType> {
        let service = service_storage
            .inner
            .insert(ServiceInner::new(init_params.driver, init_params.config));
        Ok((
            Self {
                inner: service,
                _phantom: PhantomData,
            },
            Runner {
                service,
                sensor: init_params.sensor_service,
                event_senders: init_params.event_senders,
            },
        ))
    }
}
