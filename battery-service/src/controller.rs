use core::future::Future;

use embassy_time::Duration;

use crate::device::{DynamicBatteryMsgs, StaticBatteryMsgs};

/// Fuel gauge hardware events
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum ControllerEvent {}

/// Fuel gauge controller trait that device drivers may use to integrate with internal messaging system
pub trait Controller: embedded_batteries_async::smart_battery::SmartBattery {
    type ControllerError;

    fn initialize(&mut self) -> impl Future<Output = Result<(), Self::ControllerError>>;
    fn get_static_data(&mut self) -> impl Future<Output = Result<StaticBatteryMsgs, Self::ControllerError>>;
    fn get_dynamic_data(&mut self) -> impl Future<Output = Result<DynamicBatteryMsgs, Self::ControllerError>>;
    fn get_device_event(&mut self) -> impl Future<Output = ControllerEvent>;
    fn ping(&mut self) -> impl Future<Output = Result<(), Self::ControllerError>>;

    fn get_timeout(&self) -> Duration {
        Duration::from_secs(60)
    }
    fn set_timeout(&mut self, duration: Duration);
}
