use embassy_sync::blocking_mutex::raw::RawMutex;
use embedded_services::{event, sync::Lockable, trace};
use embedded_usb_pd::{Error, LocalPortId, PdError};

use type_c_interface::port::event::VdmNotification;
use type_c_interface::port::{Controller, event::VdmData};
use type_c_interface::service::event::{PortEvent, PortEventData};

use super::{ControllerWrapper, message::vdm::Output};

impl<'device, M: RawMutex, D: Lockable, S: event::Sender<power_policy_interface::psu::event::EventData>>
    ControllerWrapper<'device, M, D, S>
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
            VdmNotification::Entered => VdmData::Entered(controller.get_other_vdm(port).await?),
            VdmNotification::Exited => VdmData::Exited(controller.get_other_vdm(port).await?),
            VdmNotification::OtherReceived => VdmData::ReceivedOther(controller.get_other_vdm(port).await?),
            VdmNotification::AttentionReceived => VdmData::ReceivedAttn(controller.get_attn_vdm(port).await?),
        };

        Ok(Output { port, vdm_data: kind })
    }

    /// Finalize a VDM output by notifying the service.
    pub(super) async fn finalize_vdm(&self, output: Output) -> Result<(), PdError> {
        trace!("Finalizing VDM output: {:?}", output);
        let Output { port, vdm_data } = output;
        let global_port_id = self.registration.pd_controller.lookup_global_port(port)?;
        self.registration
            .context
            .send_port_event(PortEvent {
                port: global_port_id,
                event: PortEventData::Vdm(vdm_data),
            })
            .await
    }
}
