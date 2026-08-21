//! Power capability definitions and related flags

/// Amount of power that a device can provider or consume
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct PowerCapability {
    /// Available voltage in mV
    pub voltage_mv: u16,
    /// Max available current in mA
    pub current_ma: u16,
}

impl PowerCapability {
    /// Calculate maximum power
    pub fn max_power_mw(&self) -> u32 {
        self.voltage_mv as u32 * self.current_ma as u32 / 1000
    }
}

impl PartialOrd for PowerCapability {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PowerCapability {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.max_power_mw().cmp(&other.max_power_mw())
    }
}

/// Power capability with consumer flags
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct ConsumerPowerCapability {
    /// Power capability
    pub capability: PowerCapability,
    /// Consumer flags
    pub flags: ConsumerFlags,
}

impl From<PowerCapability> for ConsumerPowerCapability {
    fn from(capability: PowerCapability) -> Self {
        Self {
            capability,
            flags: ConsumerFlags::default(),
        }
    }
}

/// Power capability with provider flags
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct ProviderPowerCapability {
    /// Power capability
    pub capability: PowerCapability,
    /// Provider flags
    pub flags: ProviderFlags,
}

impl From<PowerCapability> for ProviderPowerCapability {
    fn from(capability: PowerCapability) -> Self {
        Self {
            capability,
            flags: ProviderFlags::default(),
        }
    }
}

/// Combined power capability with flags enum
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum PowerCapabilityFlags {
    /// Consumer flags
    Consumer(ConsumerPowerCapability),
    /// Provider flags
    Provider(ProviderPowerCapability),
}

/// PSU type
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[non_exhaustive]
pub enum PsuType {
    /// Type-C port
    TypeC,
    /// DC barrel jack
    DcJack,

    /// Application defined type
    Custom0,
    /// Application defined type
    Custom1,
    /// Application defined type
    Custom2,
    /// Application defined type
    Custom3,
}

/// Consumer flags
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct ConsumerFlags {
    /// Unconstrained power, indicates that we are drawing power from something like an outlet and not a limited source like a battery
    pub unconstrained_power: bool,
    /// PSU type
    pub psu_type: Option<PsuType>,
}

/// Provider flags
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct ProviderFlags {
    /// PSU type
    pub psu_type: Option<PsuType>,
}

/// Consumer disconnect flags
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[non_exhaustive]
pub enum DisconnectReason {
    /// The device has been physically detached
    Detached,
    /// Switching to a different PSU
    Switching,
    /// Renegotiation triggered by the device
    AutoRenegotiation,
    /// Renegotiation triggered by code
    ManualRenegotiation,
    /// The device has changed its role
    RoleSwap,
    /// The device experienced a reset
    Reset,
}

impl DisconnectReason {
    /// Check if the reason is a renegotiation
    pub fn is_renegotiation(&self) -> bool {
        matches!(self, Self::AutoRenegotiation | Self::ManualRenegotiation)
    }
}

/// Disconnection flags
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct DisconnectFlags {
    /// Reason for the disconnect, if given
    pub reason: Option<DisconnectReason>,
}
