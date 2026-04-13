use crate::fan::Config;
use embedded_fans_async::{Error, ErrorKind, ErrorType, Fan, RpmSense};
use thermal_service_interface::fan as fan_interface;

/// `MockFan` error.
#[derive(Clone, Copy, Debug)]
pub struct MockFanError;
impl Error for MockFanError {
    fn kind(&self) -> ErrorKind {
        ErrorKind::Other
    }
}

/// Mock fan.
#[derive(Clone, Copy, Debug, Default)]
pub struct MockFan {
    rpm: u16,
}

impl MockFan {
    /// Create a new `MockFan`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns a suitable `Config` for a mock fan service.
    pub fn config() -> Config {
        Config {
            min_temp: super::MIN_TEMP + super::TEMP_RANGE / 4.0,
            ramp_temp: super::MIN_TEMP + super::TEMP_RANGE / 2.0,
            max_temp: super::MAX_TEMP - super::TEMP_RANGE / 4.0,
            ..Default::default()
        }
    }
}

impl ErrorType for MockFan {
    type Error = MockFanError;
}

impl Fan for MockFan {
    fn min_rpm(&self) -> u16 {
        0
    }

    fn max_rpm(&self) -> u16 {
        6000
    }

    fn min_start_rpm(&self) -> u16 {
        1000
    }

    async fn set_speed_rpm(&mut self, rpm: u16) -> Result<u16, Self::Error> {
        self.rpm = rpm;
        Ok(rpm)
    }
}

impl RpmSense for MockFan {
    async fn rpm(&mut self) -> Result<u16, Self::Error> {
        // The mock fan is simple, it just remembers the last RPM it was set to and reports that
        // as its current RPM.
        Ok(self.rpm)
    }
}

impl fan_interface::Driver for MockFan {}
