//! Thermal service
use crate::fan;
use crate::sensor;
use core::sync::atomic::{AtomicBool, Ordering};
use embassy_sync::once_lock::OnceLock;
use embedded_services::{error, intrusive_list};

/// Error type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Error {
    /// The requested device does not exist
    InvalidDevice,
}

struct Context {
    /// Registered temperature sensors
    sensors: intrusive_list::IntrusiveList,
    /// Registered fans
    fans: intrusive_list::IntrusiveList,
}

impl Context {
    fn new() -> Self {
        Self {
            sensors: intrusive_list::IntrusiveList::new(),
            fans: intrusive_list::IntrusiveList::new(),
        }
    }
}

static CONTEXT: OnceLock<Context> = OnceLock::new();

/// Init thermal service context
pub fn init() {
    CONTEXT.get_or_init(Context::new);
}

/// Register a sensor with the thermal service
pub async fn register_sensor(sensor: &'static sensor::DeviceNode) -> Result<(), intrusive_list::Error> {
    if get_sensor(sensor.device.id()).await.is_some() {
        return Err(intrusive_list::Error::NodeAlreadyInList);
    }

    CONTEXT.get().await.sensors.push(sensor)
}

/// Register a fan with the thermal service
pub async fn register_fan(fan: &'static fan::Device) -> Result<(), intrusive_list::Error> {
    if get_fan(fan.id()).await.is_some() {
        return Err(intrusive_list::Error::NodeAlreadyInList);
    }

    CONTEXT.get().await.fans.push(fan)
}

/// Find a sensor by its ID
async fn get_sensor(id: sensor::DeviceId) -> Option<&'static sensor::Device> {
    for sensor in &CONTEXT.get().await.sensors {
        if let Some(data) = sensor.data::<sensor::DeviceNode>() {
            if data.device.id() == id {
                return Some(data.device);
            }
        } else {
            error!("Non-device located in sensors list");
        }
    }

    None
}

/// Find a fan by its ID
async fn get_fan(id: fan::DeviceId) -> Option<&'static fan::Device> {
    for fan in &CONTEXT.get().await.fans {
        if let Some(data) = fan.data::<fan::Device>() {
            if data.id() == id {
                return Some(data);
            }
        } else {
            error!("Non-device located in fan list");
        }
    }

    None
}

/// Singleton struct to give access to the thermal context
pub struct ContextToken(());
impl ContextToken {
    /// Create a new context token, returning None if this function has been called before
    pub fn create() -> Option<Self> {
        static INIT: AtomicBool = AtomicBool::new(false);
        if INIT.load(Ordering::SeqCst) {
            return None;
        }

        INIT.store(true, Ordering::SeqCst);
        Some(ContextToken(()))
    }

    /// Get a sensor by its ID
    pub async fn get_sensor(&self, id: sensor::DeviceId) -> Result<&'static sensor::Device, Error> {
        get_sensor(id).await.ok_or(Error::InvalidDevice)
    }

    /// Provides access to the sensors list
    pub async fn sensors(&self) -> &intrusive_list::IntrusiveList {
        &CONTEXT.get().await.sensors
    }

    /// Get a fan by its ID
    pub async fn get_fan(&self, id: fan::DeviceId) -> Result<&'static fan::Device, Error> {
        get_fan(id).await.ok_or(Error::InvalidDevice)
    }

    /// Provides access to the fans list
    pub async fn fans(&self) -> &intrusive_list::IntrusiveList {
        &CONTEXT.get().await.fans
    }
}
