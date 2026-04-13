use crate::utils::SampleBuf;
use core::marker::PhantomData;
use embassy_sync::{mutex::Mutex, signal::Signal};
use embassy_time::{Duration, Timer, with_timeout};
use embedded_sensors_hal_async::temperature::DegreesCelsius;
use embedded_services::event::Sender;
use embedded_services::{GlobalRawMutex, error};
use thermal_service_interface::sensor;

// Timeout period for physical bus access
const BUS_TIMEOUT: Duration = Duration::from_millis(200);

/* Helper macro for calling a bus function with automatic retry after timeout or failure.
 *
 * Necessary since often the sensor bus is shared and occasionally the underlying bus driver
 * gets in a bad state and can hang or report spurious errors.
 */
macro_rules! with_retry {
    (
        $self:expr,
        $bus_method:expr
    ) => {{
        let mut retry_attempts = $self.config.lock().await.retry_attempts;

        loop {
            if retry_attempts == 0 {
                break Err(sensor::Error::RetryExhausted);
            }

            match with_timeout(BUS_TIMEOUT, $bus_method).await {
                Ok(Ok(val)) => break Ok(val),
                _ => {
                    retry_attempts -= 1;
                }
            }
        }
    }};
}

/// Sensor service configuration parameters.
#[derive(Clone, Copy, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Config {
    /// Rate at which to sample the sensor when operating in normal conditions.
    pub sample_period: Duration,
    /// Rate at which to sample the sensor when operating in fast conditions.
    pub fast_sample_period: Duration,
    /// Whether periodic sampling is enabled.
    pub sampling_enabled: bool,
    /// Hysteresis value to prevent rapid generation of threshold events when temperature is near a threshold.
    pub hysteresis: DegreesCelsius,
    /// Temperature threshold below which a warning event will be generated.
    pub warn_low_threshold: DegreesCelsius,
    /// Temperature threshold above which a warning event will be generated.
    pub warn_high_threshold: DegreesCelsius,
    /// Temperature threshold above which a prochot event will be generated.
    pub prochot_threshold: DegreesCelsius,
    /// Temperature threshold above which a critical event will be generated.
    pub critical_threshold: DegreesCelsius,
    /// Temperature threshold above which fast sampling is enabled.
    pub fast_sampling_threshold: DegreesCelsius,
    /// Offset to be applied to the temperature readings.
    pub offset: DegreesCelsius,
    /// Number of retry attempts for bus operations.
    pub retry_attempts: u8,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            sample_period: Duration::from_secs(1),
            fast_sample_period: Duration::from_millis(200),
            sampling_enabled: true,
            hysteresis: 2.0,
            warn_low_threshold: DegreesCelsius::MIN,
            warn_high_threshold: DegreesCelsius::MAX,
            prochot_threshold: DegreesCelsius::MAX,
            critical_threshold: DegreesCelsius::MAX,
            fast_sampling_threshold: DegreesCelsius::MAX,
            offset: 0.0,
            retry_attempts: 5,
        }
    }
}

struct ServiceInner<T: sensor::Driver, const SAMPLE_BUF_LEN: usize> {
    driver: Mutex<GlobalRawMutex, T>,
    en_signal: Signal<GlobalRawMutex, ()>,
    config: Mutex<GlobalRawMutex, Config>,
    samples: Mutex<GlobalRawMutex, SampleBuf<DegreesCelsius, SAMPLE_BUF_LEN>>,
}

impl<T: sensor::Driver, const SAMPLE_BUF_LEN: usize> ServiceInner<T, SAMPLE_BUF_LEN> {
    fn new(driver: T, config: Config) -> Self {
        Self {
            driver: Mutex::new(driver),
            en_signal: Signal::new(),
            config: Mutex::new(config),
            samples: Mutex::new(SampleBuf::create()),
        }
    }
}

/// Sensor service control handle.
pub struct Service<'hw, T: sensor::Driver, E: Sender<sensor::Event>, const SAMPLE_BUF_LEN: usize> {
    inner: &'hw ServiceInner<T, SAMPLE_BUF_LEN>,
    _phantom: PhantomData<E>,
}

// Note: We can't derive these traits because the compiler thinks our generics then need to be Copy + Clone,
// but we only hold a reference and don't actually need to be that strict
impl<T: sensor::Driver, E: Sender<sensor::Event>, const SAMPLE_BUF_LEN: usize> Clone
    for Service<'_, T, E, SAMPLE_BUF_LEN>
{
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: sensor::Driver, E: Sender<sensor::Event>, const SAMPLE_BUF_LEN: usize> Copy
    for Service<'_, T, E, SAMPLE_BUF_LEN>
{
}

impl<'hw, T: sensor::Driver, E: Sender<sensor::Event>, const SAMPLE_BUF_LEN: usize> sensor::SensorService
    for Service<'hw, T, E, SAMPLE_BUF_LEN>
{
    async fn temperature(&self) -> DegreesCelsius {
        self.inner.samples.lock().await.recent()
    }

    async fn temperature_average(&self) -> DegreesCelsius {
        self.inner.samples.lock().await.average()
    }

    async fn temperature_immediate(&self) -> Result<DegreesCelsius, sensor::Error> {
        with_retry!(self.inner, self.inner.driver.lock().await.temperature())
    }

    async fn set_threshold(&self, threshold: sensor::Threshold, value: DegreesCelsius) {
        let mut config = self.inner.config.lock().await;
        match threshold {
            sensor::Threshold::WarnLow => config.warn_low_threshold = value,
            sensor::Threshold::WarnHigh => config.warn_high_threshold = value,
            sensor::Threshold::Prochot => config.prochot_threshold = value,
            sensor::Threshold::Critical => config.critical_threshold = value,
        }
    }

    async fn threshold(&self, threshold: sensor::Threshold) -> DegreesCelsius {
        let config = self.inner.config.lock().await;
        match threshold {
            sensor::Threshold::WarnLow => config.warn_low_threshold,
            sensor::Threshold::WarnHigh => config.warn_high_threshold,
            sensor::Threshold::Prochot => config.prochot_threshold,
            sensor::Threshold::Critical => config.critical_threshold,
        }
    }

    async fn set_sample_period(&self, period: Duration) {
        self.inner.config.lock().await.sample_period = period;
    }

    async fn enable_sampling(&self) {
        self.inner.config.lock().await.sampling_enabled = true;
        self.inner.en_signal.signal(());
    }

    async fn disable_sampling(&self) {
        self.inner.config.lock().await.sampling_enabled = false;
    }
}

/// Parameters required to initialize a sensor service.
pub struct InitParams<'hw, T: sensor::Driver, E: Sender<sensor::Event>> {
    /// The underlying sensor driver this service will control.
    pub driver: T,
    /// Initial configuration for the sensor service.
    pub config: Config,
    /// Event senders for sensor events.
    pub event_senders: &'hw mut [E],
}

/// The memory resources required by the sensor.
pub struct Resources<T: sensor::Driver, const SAMPLE_BUF_LEN: usize> {
    inner: Option<ServiceInner<T, SAMPLE_BUF_LEN>>,
}

// Note: We can't derive Default unless we trait bound T by Default,
// but we don't want that restriction since the default is just the None case
impl<T: sensor::Driver, const SAMPLE_BUF_LEN: usize> Default for Resources<T, SAMPLE_BUF_LEN> {
    fn default() -> Self {
        Self { inner: None }
    }
}

// Additional sensor runner state
#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
struct State {
    is_warn_low: bool,
    is_warn_high: bool,
    is_prochot: bool,
    is_critical: bool,
}

/// A task runner for a sensor. Users must run this in an embassy task or similar async execution context.
pub struct Runner<'hw, T: sensor::Driver, E: Sender<sensor::Event>, const SAMPLE_BUF_LEN: usize> {
    service: &'hw ServiceInner<T, SAMPLE_BUF_LEN>,
    event_senders: &'hw mut [E],
    state: State,
}

impl<'hw, T: sensor::Driver, E: Sender<sensor::Event>, const SAMPLE_BUF_LEN: usize> Runner<'hw, T, E, SAMPLE_BUF_LEN> {
    async fn broadcast_event(&mut self, event: sensor::Event) {
        for sender in self.event_senders.iter_mut() {
            sender.send(event).await;
        }
    }

    async fn check_thresholds(&mut self, temp: DegreesCelsius) {
        let config = *self.service.config.lock().await;

        if temp >= config.warn_high_threshold && !self.state.is_warn_high {
            self.state.is_warn_high = true;
            self.broadcast_event(sensor::Event::ThresholdExceeded(sensor::Threshold::WarnHigh))
                .await;
        } else if temp < (config.warn_high_threshold - config.hysteresis) && self.state.is_warn_high {
            self.state.is_warn_high = false;
            self.broadcast_event(sensor::Event::ThresholdCleared(sensor::Threshold::WarnHigh))
                .await;
        }

        if temp <= config.warn_low_threshold && !self.state.is_warn_low {
            self.state.is_warn_low = true;
            self.broadcast_event(sensor::Event::ThresholdExceeded(sensor::Threshold::WarnLow))
                .await;
        } else if temp > (config.warn_low_threshold + config.hysteresis) && self.state.is_warn_low {
            self.state.is_warn_low = false;
            self.broadcast_event(sensor::Event::ThresholdCleared(sensor::Threshold::WarnLow))
                .await;
        }

        if temp >= config.prochot_threshold && !self.state.is_prochot {
            self.state.is_prochot = true;
            self.broadcast_event(sensor::Event::ThresholdExceeded(sensor::Threshold::Prochot))
                .await;
        } else if temp < (config.prochot_threshold - config.hysteresis) && self.state.is_prochot {
            self.state.is_prochot = false;
            self.broadcast_event(sensor::Event::ThresholdCleared(sensor::Threshold::Prochot))
                .await;
        }

        if temp >= config.critical_threshold && !self.state.is_critical {
            self.state.is_critical = true;
            self.broadcast_event(sensor::Event::ThresholdExceeded(sensor::Threshold::Critical))
                .await;
        } else if temp < (config.critical_threshold - config.hysteresis) && self.state.is_critical {
            self.state.is_critical = false;
            self.broadcast_event(sensor::Event::ThresholdCleared(sensor::Threshold::Critical))
                .await;
        }
    }
}

impl<'hw, T: sensor::Driver, E: Sender<sensor::Event>, const SAMPLE_BUF_LEN: usize>
    odp_service_common::runnable_service::ServiceRunner<'hw> for Runner<'hw, T, E, SAMPLE_BUF_LEN>
{
    async fn run(mut self) -> embedded_services::Never {
        loop {
            let config = *self.service.config.lock().await;

            // Only sample temperature if enabled
            if config.sampling_enabled {
                let temp = match with_retry!(self.service, self.service.driver.lock().await.temperature()) {
                    Ok(temp) => temp,
                    Err(e) => {
                        self.service.config.lock().await.sampling_enabled = false;
                        self.broadcast_event(sensor::Event::Failure(e)).await;
                        error!("Error sampling sensor, disabling sampling");
                        continue;
                    }
                };

                // Add offset to measured temperature
                let temp = temp + config.offset;

                // Cache in buffer for quick retrieval from other services
                self.service.samples.lock().await.push(temp);

                // Check thresholds
                self.check_thresholds(temp).await;

                // Adjust sampling rate based on how hot we are getting
                let sleep_duration = if temp >= config.fast_sampling_threshold {
                    config.fast_sample_period
                } else {
                    config.sample_period
                };

                // Sleep in-between sampling periods
                Timer::after(sleep_duration).await;

            // Otherwise sleep and wait to be re-enabled
            } else {
                self.service.en_signal.wait().await;
            }
        }
    }
}

impl<'hw, T: sensor::Driver, E: Sender<sensor::Event> + 'hw, const SAMPLE_BUF_LEN: usize>
    odp_service_common::runnable_service::Service<'hw> for Service<'hw, T, E, SAMPLE_BUF_LEN>
{
    type Runner = Runner<'hw, T, E, SAMPLE_BUF_LEN>;
    type Resources = Resources<T, SAMPLE_BUF_LEN>;
    type ErrorType = sensor::Error;
    type InitParams = InitParams<'hw, T, E>;

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
                event_senders: init_params.event_senders,
                state: State::default(),
            },
        ))
    }
}
