//! Type-C service utility functions and constants.
use embedded_usb_pd::pdo::{Common, Contract};
use embedded_usb_pd::type_c;
use embedded_usb_pd::{Error as PdBusError, PdError};
use fw_update_interface::basic::Error as BasicFwError;

pub fn power_capability_try_from_contract(
    contract: Contract,
) -> Option<power_policy_interface::capability::PowerCapability> {
    Some(power_policy_interface::capability::PowerCapability {
        voltage_mv: contract.pdo.max_voltage_mv(),
        current_ma: contract.operating_current_ma()?,
    })
}

pub fn power_capability_from_current(current: type_c::Current) -> power_policy_interface::capability::PowerCapability {
    power_policy_interface::capability::PowerCapability {
        voltage_mv: 5000,
        // Assume higher power for now
        current_ma: current.to_ma(false),
    }
}

/// Type-C USB2 power capability 5V@500mA
pub const POWER_CAPABILITY_USB_DEFAULT_USB2: power_policy_interface::capability::PowerCapability =
    power_policy_interface::capability::PowerCapability {
        voltage_mv: 5000,
        current_ma: 500,
    };

/// Type-C USB3 power capability 5V@900mA
pub const POWER_CAPABILITY_USB_DEFAULT_USB3: power_policy_interface::capability::PowerCapability =
    power_policy_interface::capability::PowerCapability {
        voltage_mv: 5000,
        current_ma: 900,
    };

/// Type-C power capability 5V@1.5A
pub const POWER_CAPABILITY_5V_1A5: power_policy_interface::capability::PowerCapability =
    power_policy_interface::capability::PowerCapability {
        voltage_mv: 5000,
        current_ma: 1500,
    };

/// Type-C power capability 5V@3A
pub const POWER_CAPABILITY_5V_3A0: power_policy_interface::capability::PowerCapability =
    power_policy_interface::capability::PowerCapability {
        voltage_mv: 5000,
        current_ma: 3000,
    };

/// Converts a PD error into a basic FW update error
pub fn basic_fw_update_error_from_pd_error(pd_error: PdError) -> BasicFwError {
    match pd_error {
        PdError::Busy => BasicFwError::Busy,
        _ => BasicFwError::Failed,
    }
}

/// Converts a PD error into a basic FW update error
pub fn basic_fw_update_error_from_pd_bus_error<BE>(pd_error: PdBusError<BE>) -> BasicFwError {
    match pd_error {
        PdBusError::Pd(pd_error) => basic_fw_update_error_from_pd_error(pd_error),
        PdBusError::Bus(_) => BasicFwError::Bus,
    }
}
