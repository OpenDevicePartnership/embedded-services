//! Type-C service
use embedded_usb_pd::pdo::{Common, Contract};
use embedded_usb_pd::type_c;

use crate::power::policy;

pub mod comms;
pub mod controller;
pub mod event;
pub mod external;

/// Controller ID
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct ControllerId(pub u8);

/// Length of the Other VDM data
pub const OTHER_VDM_LEN: usize = 29;
/// Length of the Attention VDM data
pub const ATTN_VDM_LEN: usize = 9;
/// Length of the Rx discover mode VDO array (in 32-bit words)
pub const DISCOVERED_MODES_LEN: usize = 7;

impl TryFrom<Contract> for policy::PowerCapability {
    type Error = ();

    fn try_from(contract: Contract) -> Result<Self, Self::Error> {
        Ok(policy::PowerCapability {
            voltage_mv: contract.pdo.max_voltage_mv(),
            current_ma: contract.operating_current_ma().ok_or(())?,
        })
    }
}

impl From<type_c::Current> for policy::PowerCapability {
    fn from(current: type_c::Current) -> Self {
        policy::PowerCapability {
            voltage_mv: 5000,
            // Assume higher power for now
            current_ma: current.to_ma(false),
        }
    }
}

/// Type-C USB2 power capability 5V@500mA
pub const POWER_CAPABILITY_USB_DEFAULT_USB2: policy::PowerCapability = policy::PowerCapability {
    voltage_mv: 5000,
    current_ma: 500,
};

/// Type-C USB3 power capability 5V@900mA
pub const POWER_CAPABILITY_USB_DEFAULT_USB3: policy::PowerCapability = policy::PowerCapability {
    voltage_mv: 5000,
    current_ma: 900,
};

/// Type-C power capability 5V@1.5A
pub const POWER_CAPABILITY_5V_1A5: policy::PowerCapability = policy::PowerCapability {
    voltage_mv: 5000,
    current_ma: 1500,
};

/// Type-C power capability 5V@3A
pub const POWER_CAPABILITY_5V_3A0: policy::PowerCapability = policy::PowerCapability {
    voltage_mv: 5000,
    current_ma: 3000,
};

/// Newtype to help clarify arguments to port status commands
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Cached(pub bool);
