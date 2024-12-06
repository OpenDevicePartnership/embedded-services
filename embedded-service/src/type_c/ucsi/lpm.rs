use crate::type_c::{Error, PortId};

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
    pub port: PortId,
    pub operation: CommandData,
}

/// LPM response data
#[derive(Copy, Clone, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum ResponseData {
    Complete,
}

impl Into<Result<ResponseData, Error>> for ResponseData {
    fn into(self) -> Result<ResponseData, Error> {
        Ok(self)
    }
}
