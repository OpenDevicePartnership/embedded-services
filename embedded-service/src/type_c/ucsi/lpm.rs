use embedded_usb_pd::PdError;

use crate::type_c::GlobalPortId;

/// Connector reset types
#[derive(Copy, Clone, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum ResetType {
    Hard,
    Data,
}

/// LPM command data
#[derive(Copy, Clone, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum CommandData {
    ConnectorReset(ResetType),
}

/// LPM commands
#[derive(Copy, Clone, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Command {
    pub port: GlobalPortId,
    pub operation: CommandData,
}

/// LPM response data
#[derive(Copy, Clone, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum ResponseData {
    Complete,
}

pub type Response = Result<ResponseData, PdError>;
