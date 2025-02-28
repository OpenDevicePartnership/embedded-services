//! Type-C service
use controller::Contract;
use embedded_usb_pd::pdo::{sink, source};
use embedded_usb_pd::type_c;

use crate::power::policy;

pub mod controller;
pub mod event;
pub mod ucsi;

/// Global port ID, used to unique identify a port
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct GlobalPortId(pub u8);

/// Controller ID
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct ControllerId(pub u8);

/// Returns the maximum power capability for an SPR AVS PDO
fn spr_avs_max_power_capability(max_current_15v_ma: u16, max_current_20v_ma: u16) -> policy::PowerCapability {
    if (max_current_15v_ma as u32 * 15000) > (max_current_20v_ma as u32 * 20000) {
        policy::PowerCapability {
            voltage_mv: 15000,
            current_ma: max_current_15v_ma,
        }
    } else {
        policy::PowerCapability {
            voltage_mv: 20000,
            current_ma: max_current_20v_ma,
        }
    }
}

impl From<source::Pdo> for policy::PowerCapability {
    fn from(pdo: source::Pdo) -> Self {
        match pdo {
            source::Pdo::Fixed(data) => policy::PowerCapability {
                voltage_mv: data.voltage_mv,
                current_ma: data.current_ma,
            },
            source::Pdo::Variable(data) => policy::PowerCapability {
                voltage_mv: data.max_voltage_mv,
                current_ma: data.max_current_ma,
            },
            source::Pdo::Battery(data) => policy::PowerCapability {
                voltage_mv: data.max_voltage_mv,
                current_ma: (data.max_power_mw / data.max_voltage_mv as u32) as u16,
            },
            source::Pdo::Augmented(apdo) => match apdo {
                source::Apdo::SprPps(data) => policy::PowerCapability {
                    voltage_mv: data.max_voltage_mv,
                    current_ma: data.max_current_ma,
                },
                source::Apdo::EprAvs(data) => policy::PowerCapability {
                    voltage_mv: data.max_voltage_mv,
                    current_ma: (data.pdp_mw / data.max_voltage_mv as u32) as u16,
                },
                source::Apdo::SprAvs(data) => {
                    spr_avs_max_power_capability(data.max_current_15v_ma, data.max_current_20v_ma)
                }
            },
        }
    }
}

impl From<sink::Pdo> for policy::PowerCapability {
    fn from(pdo: sink::Pdo) -> Self {
        match pdo {
            sink::Pdo::Fixed(data) => policy::PowerCapability {
                voltage_mv: data.voltage_mv,
                current_ma: data.operational_current_ma,
            },
            sink::Pdo::Variable(data) => policy::PowerCapability {
                voltage_mv: data.max_voltage_mv,
                current_ma: data.operational_current_ma,
            },
            sink::Pdo::Battery(data) => policy::PowerCapability {
                voltage_mv: data.max_voltage_mv,
                current_ma: (data.operational_power_mw / data.max_voltage_mv as u32) as u16,
            },
            sink::Pdo::Augmented(apdo) => match apdo {
                sink::Apdo::SprPps(data) => policy::PowerCapability {
                    voltage_mv: data.max_voltage_mv,
                    current_ma: data.max_current_ma,
                },
                sink::Apdo::EprAvs(data) => policy::PowerCapability {
                    voltage_mv: data.max_voltage_mv,
                    current_ma: (data.pdp_mw / data.max_voltage_mv as u32) as u16,
                },
                sink::Apdo::SprAvs(data) => {
                    spr_avs_max_power_capability(data.max_current_15v_ma, data.max_current_20v_ma)
                }
            },
        }
    }
}

impl From<type_c::Current> for policy::PowerCapability {
    fn from(current: type_c::Current) -> Self {
        policy::PowerCapability {
            voltage_mv: 5000,
            // Assume lower power for now
            current_ma: current.to_ma(true),
        }
    }
}

impl From<Contract> for policy::PowerCapability {
    fn from(contract: Contract) -> Self {
        match contract {
            Contract::Sink(pdo) => pdo,
            Contract::Source(pdo) => pdo,
        }
    }
}

impl From<sink::Pdo> for Contract {
    fn from(pdo: sink::Pdo) -> Self {
        Contract::Sink(policy::PowerCapability::from(pdo))
    }
}

impl From<source::Pdo> for Contract {
    fn from(pdo: source::Pdo) -> Self {
        Contract::Source(policy::PowerCapability::from(pdo))
    }
}
