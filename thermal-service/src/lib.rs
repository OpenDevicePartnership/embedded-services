//! Thermal service
#![no_std]

use thermal_service_interface::{fan::FanService, sensor::SensorService};

pub mod fan;
#[cfg(feature = "mock")]
pub mod mock;
pub mod sensor;
mod utils;

struct ServiceInner<'hw, S: SensorService, F: FanService> {
    sensors: &'hw [S],
    fans: &'hw [F],
}

/// Thermal service handle.
///
/// This maintains a list of registered temperature sensors and fans, which can be accessed by instance ID.
///
/// To allow for a collection of sensors and fans of different underlying driver types,
/// type erasure will need to be handled by the user, likely via enum dispatch,
/// since async traits are not currently dyn compatible.
#[derive(Clone, Copy)]
pub struct Service<'hw, S: SensorService, F: FanService> {
    inner: &'hw ServiceInner<'hw, S, F>,
}

/// Parameters required to initialize the thermal service.
pub struct InitParams<'hw, S: SensorService, F: FanService> {
    /// Registered temperature sensors.
    pub sensors: &'hw [S],
    /// Registered fans.
    pub fans: &'hw [F],
}

/// The memory resources required by the thermal service.
pub struct Resources<'hw, S: SensorService, F: FanService> {
    inner: Option<ServiceInner<'hw, S, F>>,
}

// Note: We can't derive Default because the compiler requires S: Default + F: Default bounds,
// but we don't need that since the default is just the None case
impl<S: SensorService, F: FanService> Default for Resources<'_, S, F> {
    fn default() -> Self {
        Self { inner: None }
    }
}

impl<'hw, S: SensorService, F: FanService> Service<'hw, S, F> {
    /// Initializes the thermal service with the provided sensors and fans.
    pub fn init(resources: &'hw mut Resources<'hw, S, F>, init_params: InitParams<'hw, S, F>) -> Self {
        let inner = resources.inner.insert(ServiceInner {
            sensors: init_params.sensors,
            fans: init_params.fans,
        });
        Self { inner }
    }
}

impl<'hw, S: SensorService + Copy, F: FanService + Copy> thermal_service_interface::ThermalService
    for Service<'hw, S, F>
{
    type Sensor = S;
    type Fan = F;

    fn sensor(&self, id: u8) -> Option<Self::Sensor> {
        self.inner.sensors.get(id as usize).copied()
    }

    fn fan(&self, id: u8) -> Option<Self::Fan> {
        self.inner.fans.get(id as usize).copied()
    }
}
