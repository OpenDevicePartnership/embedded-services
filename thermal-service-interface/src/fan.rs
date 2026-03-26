use core::future::Future;
use embassy_time::Duration;
use embedded_fans_async::{Fan, RpmSense};
use embedded_sensors_hal_async::temperature::DegreesCelsius;

/// Ensures all necessary traits are implemented for the underlying fan driver.
pub trait Driver: Fan + RpmSense {}

/// Fan error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[non_exhaustive]
pub enum Error {
    /// Fan encountered a hardware failure.
    Hardware,
}

/// Fan event.
#[derive(Debug, PartialEq, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[non_exhaustive]
pub enum Event {
    /// Fan encountered a failure.
    Failure(Error),
}

/// Fan on (running) state.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum OnState {
    /// Fan is on and running at its minimum speed.
    Min,
    /// Fan is ramping up or down along a curve in response to a temperature change.
    Ramping,
    /// Fan is running at its maximum speed.
    Max,
}

/// Fan state.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum State {
    /// Fan is off.
    Off,
    /// Fan is on in the specified [`OnState`].
    On(OnState),
}

/// Fan service interface trait.
pub trait FanService {
    /// Enable automatic fan control.
    ///
    /// This allows the fan to automatically change [`State`] based on periodic readings from an associated temperature sensor.
    fn enable_auto_control(&self) -> impl Future<Output = Result<(), Error>>;
    /// Returns the most recently sampled RPM measurement.
    fn rpm(&self) -> impl Future<Output = u16>;
    /// Returns the minimum RPM supported by the fan.
    fn min_rpm(&self) -> impl Future<Output = u16>;
    /// Returns the maximum RPM supported by the fan.
    fn max_rpm(&self) -> impl Future<Output = u16>;
    /// Returns the average RPM over a sampling period.
    fn rpm_average(&self) -> impl Future<Output = u16>;
    /// Immediately samples the fan for an RPM measurement and returns the result.
    fn rpm_immediate(&self) -> impl Future<Output = Result<u16, Error>>;
    /// Sets the fan to run at the specified RPM (and disables automatic control).
    fn set_rpm(&self, rpm: u16) -> impl Future<Output = Result<(), Error>>;
    /// Sets the fan to run at the specified duty cycle percentage (and disables automatic control).
    fn set_duty_percent(&self, duty: u8) -> impl Future<Output = Result<(), Error>>;
    /// Stops the fan (and disables automatic control).
    fn stop(&self) -> impl Future<Output = Result<(), Error>>;
    /// Set the rate at which RPM measurements are sampled.
    fn set_rpm_sampling_period(&self, period: Duration) -> impl Future<Output = ()>;
    /// Set the rate at which the fan will update its RPM in response to a temperature change when in automatic control mode.
    fn set_rpm_update_period(&self, period: Duration) -> impl Future<Output = ()>;
    /// Returns the temperature at which the fan will change to the specified [`OnState`] when in automatic control mode.
    fn state_temp(&self, state: OnState) -> impl Future<Output = DegreesCelsius>;
    /// Sets the temperature at which the fan will change to the specified [`OnState`] when in automatic control mode.
    fn set_state_temp(&self, state: OnState, temp: DegreesCelsius) -> impl Future<Output = ()>;
}

impl<T: FanService> FanService for &T {
    fn enable_auto_control(&self) -> impl Future<Output = Result<(), Error>> {
        T::enable_auto_control(self)
    }

    fn rpm(&self) -> impl Future<Output = u16> {
        T::rpm(self)
    }

    fn min_rpm(&self) -> impl Future<Output = u16> {
        T::min_rpm(self)
    }

    fn max_rpm(&self) -> impl Future<Output = u16> {
        T::max_rpm(self)
    }

    fn rpm_average(&self) -> impl Future<Output = u16> {
        T::rpm_average(self)
    }

    fn rpm_immediate(&self) -> impl Future<Output = Result<u16, Error>> {
        T::rpm_immediate(self)
    }

    fn set_rpm(&self, rpm: u16) -> impl Future<Output = Result<(), Error>> {
        T::set_rpm(self, rpm)
    }

    fn set_duty_percent(&self, duty: u8) -> impl Future<Output = Result<(), Error>> {
        T::set_duty_percent(self, duty)
    }

    fn stop(&self) -> impl Future<Output = Result<(), Error>> {
        T::stop(self)
    }

    fn set_rpm_sampling_period(&self, period: Duration) -> impl Future<Output = ()> {
        T::set_rpm_sampling_period(self, period)
    }

    fn set_rpm_update_period(&self, period: Duration) -> impl Future<Output = ()> {
        T::set_rpm_update_period(self, period)
    }

    fn state_temp(&self, state: OnState) -> impl Future<Output = DegreesCelsius> {
        T::state_temp(self, state)
    }

    fn set_state_temp(&self, state: OnState, temp: DegreesCelsius) -> impl Future<Output = ()> {
        T::set_state_temp(self, state, temp)
    }
}
