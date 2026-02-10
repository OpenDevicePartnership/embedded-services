//! Type-C service
use embedded_usb_pd::pdo::{Common, Contract};
use embedded_usb_pd::type_c;

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

pub fn power_capability_try_from_contract(
    contract: Contract,
) -> Option<power_policy_service::capability::PowerCapability> {
    Some(power_policy_service::capability::PowerCapability {
        voltage_mv: contract.pdo.max_voltage_mv(),
        current_ma: contract.operating_current_ma()?,
    })
}

pub fn power_capability_from_current(current: type_c::Current) -> power_policy_service::capability::PowerCapability {
    power_policy_service::capability::PowerCapability {
        voltage_mv: 5000,
        // Assume higher power for now
        current_ma: current.to_ma(false),
    }
}

/// Type-C USB2 power capability 5V@500mA
pub const POWER_CAPABILITY_USB_DEFAULT_USB2: power_policy_service::capability::PowerCapability =
    power_policy_service::capability::PowerCapability {
        voltage_mv: 5000,
        current_ma: 500,
    };

/// Type-C USB3 power capability 5V@900mA
pub const POWER_CAPABILITY_USB_DEFAULT_USB3: power_policy_service::capability::PowerCapability =
    power_policy_service::capability::PowerCapability {
        voltage_mv: 5000,
        current_ma: 900,
    };

/// Type-C power capability 5V@1.5A
pub const POWER_CAPABILITY_5V_1A5: power_policy_service::capability::PowerCapability =
    power_policy_service::capability::PowerCapability {
        voltage_mv: 5000,
        current_ma: 1500,
    };

/// Type-C power capability 5V@3A
pub const POWER_CAPABILITY_5V_3A0: power_policy_service::capability::PowerCapability =
    power_policy_service::capability::PowerCapability {
        voltage_mv: 5000,
        current_ma: 3000,
    };

/// Newtype to help clarify arguments to port status commands
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Cached(pub bool);
