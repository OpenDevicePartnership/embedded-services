pub mod fan;
pub mod sensor;

const SAMPLE_BUF_LEN: usize = 16;

// Represents the temperature ranges the mock thermal service will move through
pub(crate) const MIN_TEMP: f32 = 20.0;
pub(crate) const MAX_TEMP: f32 = 40.0;
pub(crate) const TEMP_RANGE: f32 = MAX_TEMP - MIN_TEMP;

/// Default mock sensor ID.
pub const MOCK_SENSOR_ID: crate::sensor::DeviceId = crate::sensor::DeviceId(0);

/// Default mock fan ID.
pub const MOCK_FAN_ID: crate::fan::DeviceId = crate::fan::DeviceId(0);

/// A thermal-service wrapped [`sensor::MockSensor`].
pub type TsMockSensor = crate::sensor::Sensor<sensor::MockSensor, SAMPLE_BUF_LEN>;

/// A thermal-service wrapped [`fan::MockFan`].
pub type TsMockFan = crate::fan::Fan<fan::MockFan, SAMPLE_BUF_LEN>;

/// Creates a new mock sensor ready for use with the thermal service.
///
/// This is a convenience wrapper, but for finer control a [`sensor::MockSensor`] can still be
/// constructed manually.
///
/// This still needs to be wrapped in a static and registered with the thermal service,
/// and then a respective task spawned.
pub fn new_sensor() -> TsMockSensor {
    let sensor = sensor::MockSensor::new();
    crate::sensor::Sensor::new(MOCK_SENSOR_ID, sensor, crate::sensor::Profile::default())
}

/// Creates a new mock fan ready for use with the thermal service.
///
/// This is a convenience wrapper, but for finer control a [`fan::MockFan`] can still be
/// constructed manually.
///
/// This still needs to be wrapped in a static and registered with the thermal service,
/// and then a respective task spawned.
pub fn new_fan() -> TsMockFan {
    let fan = fan::MockFan::new();

    // Attaches the mock sensor to the mock fan and set the fan state temps
    // so that they are in range with the mock sensor
    let profile = crate::fan::Profile {
        sensor_id: MOCK_SENSOR_ID,
        auto_control: true,
        on_temp: MIN_TEMP + TEMP_RANGE / 4.0,
        ramp_temp: MIN_TEMP + TEMP_RANGE / 2.0,
        max_temp: MAX_TEMP - TEMP_RANGE / 4.0,
        ..Default::default()
    };

    crate::fan::Fan::new(MOCK_FAN_ID, fan, profile)
}
