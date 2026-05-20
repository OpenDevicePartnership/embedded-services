#![no_std]

pub mod fan;
pub mod sensor;

/// Thermal service event.
#[derive(Debug, PartialEq, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[non_exhaustive]
pub enum Event {
    /// A sensor event occurred.
    Sensor(u8, sensor::Event),
    /// A fan event occurred.
    Fan(u8, fan::Event),
}

/// Thermal service interface trait.
pub trait ThermalService {
    /// Associated type for registered sensor services.
    type Sensor: sensor::SensorService;
    /// Associated type for registered fan services.
    type Fan: fan::FanService;

    /// Retrieve a handle to the sensor service with the specified instance ID, if it exists.
    fn sensor(&self, id: u8) -> Option<Self::Sensor>;
    /// Retrieve a handle to the fan service with the specified instance ID, if it exists.
    fn fan(&self, id: u8) -> Option<Self::Fan>;
}
