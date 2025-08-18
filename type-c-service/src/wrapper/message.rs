//! [`crate::wrapper::ControllerWrapper`] message types
use embedded_services::{
    ipc::deferred,
    power::policy,
    type_c::{
        controller::{self, PortStatus},
        event::{PortNotificationSingle, PortStatusChanged},
    },
    GlobalRawMutex,
};
use embedded_usb_pd::{ado::Ado, PortId as LocalPortId};

/// Port status changed event data
#[derive(Copy, Clone, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct EventPortStatusChanged {
    /// Port ID
    pub port: LocalPortId,
    /// Status changed event
    pub status_event: PortStatusChanged,
}

/// Port notification event data
#[derive(Copy, Clone, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct EventPortNotification {
    /// Port ID
    pub port: LocalPortId,
    /// Notification event
    pub notification: PortNotificationSingle,
}

/// Power policy command event data
pub struct EventPowerPolicyCommand<'a> {
    /// Port ID
    pub port: LocalPortId,
    /// Power policy request
    pub request:
        deferred::Request<'a, GlobalRawMutex, policy::device::CommandData, policy::device::InternalResponseData>,
}

/// CFU events
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum EventCfu {
    /// CFU request
    Request(embedded_services::cfu::component::RequestData),
    /// Recovery tick
    ///
    /// Occurs when the FW update has timed out to abort the update and return hardware to its normal state
    RecoveryTick,
}

/// Wrapper events
pub enum Event<'a> {
    /// Port status changed
    PortStatusChanged(EventPortStatusChanged),
    /// Port notification
    PortNotification(EventPortNotification),
    /// Power policy command received
    PowerPolicyCommand(EventPowerPolicyCommand<'a>),
    /// Command from TCPM
    ControllerCommand(deferred::Request<'a, GlobalRawMutex, controller::Command, controller::Response<'static>>),
    /// Cfu event
    CfuEvent(EventCfu),
}

/// Port status changed output data
#[derive(Copy, Clone, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct OutputPortStatusChanged {
    /// Port ID
    pub port: LocalPortId,
    /// Status changed event
    pub status_event: PortStatusChanged,
    /// Port status
    pub status: PortStatus,
}

/// PD alert output data
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct OutputPdAlert {
    /// Port ID
    pub port: LocalPortId,
    /// ADO data
    pub ado: Ado,
}

/// Power policy command output data
pub struct OutputPowerPolicyCommand<'a> {
    /// Port ID
    pub port: LocalPortId,
    /// Power policy request
    pub request:
        deferred::Request<'a, GlobalRawMutex, policy::device::CommandData, policy::device::InternalResponseData>,
    /// Response
    pub response: policy::device::InternalResponseData,
}

/// Controller command output data
pub struct OutputControllerCommand<'a> {
    /// Controller request
    pub request: deferred::Request<'a, GlobalRawMutex, controller::Command, controller::Response<'static>>,
    /// Response
    pub response: controller::Response<'static>,
}

/// [`crate::wrapper::ControllerWrapper`] output
pub enum Output<'a> {
    /// No-op when nothing specific is needed
    Nop,
    /// Port status changed
    PortStatusChanged(OutputPortStatusChanged),
    /// PD alert
    PdAlert(OutputPdAlert),
    /// Power policy command received
    PowerPolicyCommand(OutputPowerPolicyCommand<'a>),
    /// TPCM command response
    ControllerCommand(OutputControllerCommand<'a>),
    /// CFU recovery tick
    CfuRecovery,
    /// CFU response
    CfuResponse(embedded_services::cfu::component::InternalResponseData),
}
