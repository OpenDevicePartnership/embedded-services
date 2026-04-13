use core::future::Future;
use embassy_time::Duration;
use embedded_sensors_hal_async::temperature::{DegreesCelsius, TemperatureSensor};

/// Ensures all necessary traits are implemented for the underlying sensor driver.
pub trait Driver: TemperatureSensor {}

/// Sensor error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[non_exhaustive]
pub enum Error {
    /// Sensor encountered a hardware failure.
    Hardware,
    /// Retry attempts to communicate with sensor exhausted.
    RetryExhausted,
}

/// Sensor event.
#[derive(Debug, PartialEq, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[non_exhaustive]
pub enum Event {
    /// A sensor threshold was exceeded.
    ThresholdExceeded(Threshold),
    /// A sensor threshold which was previously exceeded is now cleared.
    ThresholdCleared(Threshold),
    /// Sensor encountered a failure.
    Failure(Error),
}

/// Sensor threshold types.
#[derive(Debug, PartialEq, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Threshold {
    /// The temperature threshold below which a warning event is generated.
    WarnLow,
    /// The temperature threshold above which a warning event is generated.
    WarnHigh,
    /// The temperature threshold above which a prochot event is generated.
    Prochot,
    /// The temperature threshold above which a critical event is generated.
    Critical,
}

/// Sensor service interface trait
pub trait SensorService {
    /// Returns the most recently sampled temperature measurement in degrees Celsius.
    fn temperature(&self) -> impl Future<Output = DegreesCelsius>;
    /// Returns the average temperature over a sampling period in degrees Celsius.
    fn temperature_average(&self) -> impl Future<Output = DegreesCelsius>;
    /// Immediately samples the sensor for a temperature measurement and returns the result in degrees Celsius.
    fn temperature_immediate(&self) -> impl Future<Output = Result<DegreesCelsius, Error>>;
    /// Sets the temperature for which a sensor event will be generated when the threshold is exceeded, in degrees Celsius.
    fn set_threshold(&self, threshold: Threshold, value: DegreesCelsius) -> impl Future<Output = ()>;
    /// Returns the temperature threshold value for the specified threshold type in degrees Celsius.
    fn threshold(&self, threshold: Threshold) -> impl Future<Output = DegreesCelsius>;
    /// Sets the rate at which temperature measurements are sampled.
    fn set_sample_period(&self, period: Duration) -> impl Future<Output = ()>;
    /// Enable periodic temperature sampling.
    fn enable_sampling(&self) -> impl Future<Output = ()>;
    /// Disable periodic temperature sampling.
    fn disable_sampling(&self) -> impl Future<Output = ()>;
}

impl<T: SensorService> SensorService for &T {
    async fn temperature(&self) -> DegreesCelsius {
        T::temperature(self).await
    }

    async fn temperature_average(&self) -> DegreesCelsius {
        T::temperature_average(self).await
    }

    async fn temperature_immediate(&self) -> Result<DegreesCelsius, Error> {
        T::temperature_immediate(self).await
    }

    async fn set_threshold(&self, threshold: Threshold, value: DegreesCelsius) {
        T::set_threshold(self, threshold, value).await
    }

    async fn threshold(&self, threshold: Threshold) -> DegreesCelsius {
        T::threshold(self, threshold).await
    }

    async fn set_sample_period(&self, period: Duration) {
        T::set_sample_period(self, period).await
    }

    async fn enable_sampling(&self) {
        T::enable_sampling(self).await
    }

    async fn disable_sampling(&self) {
        T::disable_sampling(self).await
    }
}
