pub mod fan;
pub mod sensor;

// Represents the temperature ranges the mock thermal service will move through
pub(crate) const MIN_TEMP: f32 = 20.0;
pub(crate) const MAX_TEMP: f32 = 40.0;
pub(crate) const TEMP_RANGE: f32 = MAX_TEMP - MIN_TEMP;
