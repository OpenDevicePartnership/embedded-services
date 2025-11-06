use embassy_sync::blocking_mutex::raw::RawMutex;
use embedded_services::{
    sync::Lockable,
    error, trace,
    type_c::{
        controller::Controller,
        event::{PortPending, VdmNotification},
    },
};
use embedded_usb_pd::{Error, LocalPortId, PdError, vdm::Svid};

use crate::wrapper::{DynPortState, message::vdm::OutputKind};

use super::{
    ControllerWrapper, FwOfferValidator,
    message::{OutputDiscModeVdos, vdm::Output},
};

impl<'device, M: RawMutex, C: Lockable, V: FwOfferValidator> ControllerWrapper<'device, M, C, V>
where
    <C as Lockable>::Inner: Controller,
{
    /// Process a VDM event by retrieving the relevant VDM data from the `controller` for the appropriate `port`.
    pub(super) async fn process_vdm_event(
        &self,
        controller: &mut C::Inner,
        port: LocalPortId,
        event: VdmNotification,
    ) -> Result<Output, Error<<C::Inner as Controller>::BusError>> {
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
    pub(super) async fn finalize_vdm(&self, state: &mut dyn DynPortState<'_>, output: Output) -> Result<(), PdError> {
        trace!("Finalizing VDM output: {:?}", output);
        let Output { port, kind } = output;
        let global_port_id = self.registration.pd_controller.lookup_global_port(port)?;
        let port_index = port.0 as usize;
        let notification = &mut state.port_states_mut()[port_index].pending_events.notification;
        match kind {
            OutputKind::Entered(_) => notification.set_custom_mode_entered(true),
            OutputKind::Exited(_) => notification.set_custom_mode_exited(true),
            OutputKind::ReceivedOther(_) => notification.set_custom_mode_other_vdm_received(true),
            OutputKind::ReceivedAttn(_) => notification.set_custom_mode_attention_received(true),
        }

        let mut pending = PortPending::none();
        pending.pend_port(global_port_id.0 as usize);
        self.registration.pd_controller.notify_ports(pending).await;
        Ok(())
    }

    /// Process a notification event by retrieving the relevant VDO data from the `controller` for the appropriate `port`.
    pub(super) async fn process_disc_mode_completed_event(
        &self,
        controller: &mut C,
        port: LocalPortId,
        svid: Svid,
    ) -> Result<OutputDiscModeVdos, Error<<C as Controller>::BusError>> {
        trace!("Processing Discover Mode Completed event on port {}", port.0);
        match controller.get_rx_disc_mode_vdos(port, svid).await {
            Ok(disc_mode_vdos) => Ok(OutputDiscModeVdos {
                port,
                vdos: disc_mode_vdos,
            }),
            Err(err) => {
                error!("Failed to get discover mode VDOs on port {}", port.0);
                Err(err)
            }
        }
    }
}
