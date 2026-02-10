use embassy_sync::blocking_mutex::raw::RawMutex;
use embedded_services::{event, sync::Lockable, trace};
use embedded_usb_pd::{Error, LocalPortId, PdError};

use crate::wrapper::{DynPortState, message::vdm::OutputKind};

use crate::type_c::{
    controller::Controller,
    event::{PortPending, VdmNotification},
};

use super::{ControllerWrapper, FwOfferValidator, message::vdm::Output};

impl<
    'device,
    M: RawMutex,
    D: Lockable,
    S: event::Sender<power_policy_service::psu::event::RequestData>,
    R: event::Receiver<power_policy_service::psu::event::RequestData>,
    V: FwOfferValidator,
> ControllerWrapper<'device, M, D, S, R, V>
where
    D::Inner: Controller,
{
    /// Process a VDM event by retrieving the relevant VDM data from the `controller` for the appropriate `port`.
    pub(super) async fn process_vdm_event(
        &self,
        controller: &mut D::Inner,
        port: LocalPortId,
        event: VdmNotification,
    ) -> Result<Output, Error<<D::Inner as Controller>::BusError>> {
        trace!("Processing VDM event: {:?} on port {}", event, port.0);
        let kind = match event {
            VdmNotification::Entered => OutputKind::Entered(controller.get_other_vdm(port).await?),
            VdmNotification::Exited => OutputKind::Exited(controller.get_other_vdm(port).await?),
            VdmNotification::OtherReceived => OutputKind::ReceivedOther(controller.get_other_vdm(port).await?),
            VdmNotification::AttentionReceived => OutputKind::ReceivedAttn(controller.get_attn_vdm(port).await?),
        };

        Ok(Output { port, kind })
    }

    /// Finalize a VDM output by notifying the service.
    pub(super) async fn finalize_vdm(
        &self,
        state: &mut dyn DynPortState<'_, S>,
        output: Output,
    ) -> Result<(), PdError> {
        trace!("Finalizing VDM output: {:?}", output);
        let Output { port, kind } = output;
        let global_port_id = self.registration.pd_controller.lookup_global_port(port)?;
        let port_index = port.0 as usize;
        let notification = &mut state
            .port_states_mut()
            .get_mut(port_index)
            .ok_or(PdError::InvalidPort)?
            .pending_events
            .notification;
        match kind {
            OutputKind::Entered(_) => notification.set_custom_mode_entered(true),
            OutputKind::Exited(_) => notification.set_custom_mode_exited(true),
            OutputKind::ReceivedOther(_) => notification.set_custom_mode_other_vdm_received(true),
            OutputKind::ReceivedAttn(_) => notification.set_custom_mode_attention_received(true),
        }

        let mut pending = PortPending::none();
        pending
            .pend_port(global_port_id.0 as usize)
            .map_err(|_| PdError::InvalidPort)?;
        self.registration
            .pd_controller
            .notify_ports(self.registration.context, pending);
        Ok(())
    }
}
