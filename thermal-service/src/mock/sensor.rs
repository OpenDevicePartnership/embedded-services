use crate::sensor;
use embedded_sensors_hal_async::sensor as sensor_traits;
use embedded_sensors_hal_async::temperature::{DegreesCelsius, TemperatureSensor, TemperatureThresholdSet};

/// `MockSensor` error.
#[derive(Clone, Copy, Debug)]
pub struct MockSensorError;
impl sensor_traits::Error for MockSensorError {
    fn kind(&self) -> sensor_traits::ErrorKind {
        sensor_traits::ErrorKind::Other
    }
}

impl sensor_traits::ErrorType for MockSensor {
    type Error = MockSensorError;
}

/// Mock sensor.
#[derive(Clone, Copy, Debug, Default)]
pub struct MockSensor {
    temp: DegreesCelsius,
    falling: bool,
}

impl MockSensor {
    /// Create a new `MockSensor`.
    pub fn new() -> Self {
        Self {
            temp: super::MIN_TEMP,
            falling: false,
        }
    }
}

impl TemperatureSensor for MockSensor {
    async fn temperature(&mut self) -> Result<DegreesCelsius, Self::Error> {
        let t = self.temp;

        // Creates a sawtooth pattern
        if self.falling {
            self.temp -= 1.0;
            if self.temp <= super::MIN_TEMP {
                self.temp = super::MIN_TEMP;
                self.falling = false;
            }
        } else {
            self.temp += 1.0;
            if self.temp >= super::MAX_TEMP {
                self.temp = super::MAX_TEMP;
                self.falling = true;
            }
        }

        Ok(t)
    }
}

// Setting a threshold for `MockSensor` doesn't make sense so immediately return error
impl TemperatureThresholdSet for MockSensor {
    async fn set_temperature_threshold_low(&mut self, _threshold: DegreesCelsius) -> Result<(), Self::Error> {
        Err(MockSensorError)
    }

    async fn set_temperature_threshold_high(&mut self, _threshold: DegreesCelsius) -> Result<(), Self::Error> {
        Err(MockSensorError)
    }
}

impl sensor::CustomRequestHandler for MockSensor {}
impl sensor::Controller for MockSensor {}
