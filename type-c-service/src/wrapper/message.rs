//! [`crate::wrapper::ControllerWrapper`] message types
use embedded_services::{GlobalRawMutex, ipc::deferred};
use embedded_usb_pd::{LocalPortId, ado::Ado};

use crate::type_c::{
    controller::{self, DpStatus, PortStatus},
    event::{PortNotificationSingle, PortStatusChanged},
};

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
pub struct EventPowerPolicyCommand {
    /// Port ID
    pub port: LocalPortId,
    /// Power policy request
    pub request: power_policy_service::device::CommandData,
}

/// CFU events
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum EventCfu {
    /// CFU request
    Request(cfu_service::component::RequestData),
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
    PowerPolicyCommand(EventPowerPolicyCommand),
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
pub struct OutputPowerPolicyCommand {
    /// Port ID
    pub port: LocalPortId,
    /// Response
    pub response: power_policy_service::device::InternalResponseData,
}

/// Controller command output data
pub struct OutputControllerCommand<'a> {
    /// Controller request
    pub request: deferred::Request<'a, GlobalRawMutex, controller::Command, controller::Response<'static>>,
    /// Response
    pub response: controller::Response<'static>,
}

pub mod vdm {
    //! Events and output for vendor-defined messaging.
    use super::LocalPortId;
    use crate::type_c::controller::{AttnVdm, OtherVdm};

    /// The kind of output from processing a vendor-defined message.
    #[derive(Copy, Clone, Debug)]
    #[cfg_attr(feature = "defmt", derive(defmt::Format))]
    pub enum OutputKind {
        /// Entered custom mode
        Entered(OtherVdm),
        /// Exited custom mode
        Exited(OtherVdm),
        /// Received a vendor-defined other message
        ReceivedOther(OtherVdm),
        /// Received a vendor-defined attention message
        ReceivedAttn(AttnVdm),
    }

    /// Output from processing a vendor-defined message.
    #[derive(Copy, Clone, Debug)]
    #[cfg_attr(feature = "defmt", derive(defmt::Format))]
    pub struct Output {
        /// The port that the VDM message is associated with.
        pub port: LocalPortId,

        /// The kind of VDM output.
        pub kind: OutputKind,
    }
}

/// DP status changed output data
#[derive(Copy, Clone, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct OutputDpStatusChanged {
    /// Port ID
    pub port: LocalPortId,
    /// Port status
    pub status: DpStatus,
}

/// [`crate::wrapper::ControllerWrapper`] output
pub enum Output<'a> {
    /// No-op when nothing specific is needed
    Nop,
    /// Port status changed
    PortStatusChanged(OutputPortStatusChanged),
    /// PD alert
    PdAlert(OutputPdAlert),
    /// Vendor-defined messaging.
    Vdm(vdm::Output),
    /// Power policy command received
    PowerPolicyCommand(OutputPowerPolicyCommand),
    /// TPCM command response
    ControllerCommand(OutputControllerCommand<'a>),
    /// CFU recovery tick
    CfuRecovery,
    /// CFU response
    CfuResponse(cfu_service::component::InternalResponseData),
    /// Dp status update
    DpStatusUpdate(OutputDpStatusChanged),
}
