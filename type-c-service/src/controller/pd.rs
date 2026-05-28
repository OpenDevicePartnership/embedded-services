//! PD functionality unrelated to power contracts and general port status
use embedded_services::{event::NonBlockingSender, sync::Lockable};
use embedded_usb_pd::PdError;
use embedded_usb_pd::ado::Ado;
use embedded_usb_pd::vdm::structured::command::discover_identity::{sop, sop_prime};
use type_c_interface::control::{
    dp::{DpConfig, DpStatus},
    pd::{PdStateMachineConfig, PortStatus},
    svid::DiscoveredSvids,
    tbt::TbtConfig,
    usb::UsbControlConfig,
    vdm::{AttnVdm, OtherVdm, SendVdm},
};
use type_c_interface::controller::pd::StateMachine;
use type_c_interface::port::event::{VdmData, VdmNotification};
use type_c_interface::service::event::PortEventData as ServicePortEventData;

use super::*;
use crate::controller::state::SharedState;

impl<
    'device,
    C: Lockable<Inner: Pd>,
    Shared: Lockable<Inner = SharedState>,
    TypeCSender: NonBlockingSender<type_c_interface::service::event::PortEventData>,
    PowerSender: NonBlockingSender<power_policy_interface::psu::event::EventData>,
    LoopbackSender: NonBlockingSender<event::Loopback>,
> Port<'device, C, Shared, TypeCSender, PowerSender, LoopbackSender>
{
    /// Process a VDM event by retrieving the relevant VDM data from the `controller` for the appropriate `port`.
    pub(super) async fn process_vdm_event(
        &mut self,
        event: VdmNotification,
    ) -> Result<Option<ServicePortEventData>, PdError> {
        debug!("({}): Processing VDM event: {:?}", self.name, event);
        let vdm_data = {
            let mut controller = self.controller.lock().await;
            match event {
                VdmNotification::Entered => VdmData::Entered(controller.get_other_vdm(self.port).await?),
                VdmNotification::Exited => VdmData::Exited(controller.get_other_vdm(self.port).await?),
                VdmNotification::OtherReceived => VdmData::ReceivedOther(controller.get_other_vdm(self.port).await?),
                VdmNotification::AttentionReceived => VdmData::ReceivedAttn(controller.get_attn_vdm(self.port).await?),
                _ => {
                    info!("({}): Received unknown VDM event: {:?}", self.name, event);
                    return Ok(None);
                }
            }
        };

        let event = ServicePortEventData::Vdm(vdm_data);
        if self.type_c_sender.try_send(event).is_none() {
            error!("Failed to send VDM type-C event");
        }
        Ok(Some(event))
    }

    /// Process a DisplayPort status update by retrieving the current DP status from the `controller` for the appropriate `port`.
    pub(super) async fn process_dp_status_update(&mut self) -> Result<ServicePortEventData, PdError> {
        debug!("({}): Processing DP status update event", self.name);
        let status = self.controller.lock().await.get_dp_status(self.port).await?;
        let event = ServicePortEventData::DpStatusUpdate(status);
        if self.type_c_sender.try_send(event).is_none() {
            error!("Failed to send DP status update type-C event");
        }
        Ok(event)
    }

    pub(super) async fn process_pd_alert(&mut self) -> Result<Option<ServicePortEventData>, PdError> {
        let ado = self.controller.lock().await.get_pd_alert(self.port).await?;
        debug!("({}): PD alert: {:#?}", self.name, ado);
        if let Some(ado) = ado {
            let event = ServicePortEventData::Alert(ado);
            if self.type_c_sender.try_send(event).is_none() {
                error!("Failed to send PD alert type-C event");
            }
            Ok(Some(event))
        } else {
            // For some reason we didn't read an alert, nothing to do
            Ok(None)
        }
    }
}

impl<
    'device,
    C: Lockable<Inner: Pd>,
    Shared: Lockable<Inner = SharedState>,
    TypeCSender: NonBlockingSender<type_c_interface::service::event::PortEventData>,
    PowerSender: NonBlockingSender<power_policy_interface::psu::event::EventData>,
    LoopbackSender: NonBlockingSender<event::Loopback>,
> type_c_interface::port::pd::Pd for Port<'device, C, Shared, TypeCSender, PowerSender, LoopbackSender>
{
    async fn get_port_status(&mut self) -> Result<PortStatus, PdError> {
        self.controller.lock().await.get_port_status(self.port).await
    }

    async fn clear_dead_battery_flag(&mut self) -> Result<(), PdError> {
        self.controller.lock().await.clear_dead_battery_flag(self.port).await
    }

    async fn enable_sink_path(&mut self, enable: bool) -> Result<(), PdError> {
        self.controller.lock().await.enable_sink_path(self.port, enable).await
    }

    async fn get_pd_alert(&mut self) -> Result<Option<Ado>, PdError> {
        self.controller.lock().await.get_pd_alert(self.port).await
    }

    async fn set_unconstrained_power(&mut self, unconstrained: bool) -> Result<(), PdError> {
        self.controller
            .lock()
            .await
            .set_unconstrained_power(self.port, unconstrained)
            .await
    }

    async fn get_other_vdm(&mut self) -> Result<OtherVdm, PdError> {
        self.controller.lock().await.get_other_vdm(self.port).await
    }

    async fn get_attn_vdm(&mut self) -> Result<AttnVdm, PdError> {
        self.controller.lock().await.get_attn_vdm(self.port).await
    }

    async fn send_vdm(&mut self, tx_vdm: SendVdm) -> Result<(), PdError> {
        self.controller.lock().await.send_vdm(self.port, tx_vdm).await
    }

    async fn execute_drst(&mut self) -> Result<(), PdError> {
        self.controller.lock().await.execute_drst(self.port).await
    }

    async fn get_dp_status(&mut self) -> Result<DpStatus, PdError> {
        self.controller.lock().await.get_dp_status(self.port).await
    }

    async fn set_dp_config(&mut self, config: DpConfig) -> Result<(), PdError> {
        self.controller.lock().await.set_dp_config(self.port, config).await
    }

    async fn set_tbt_config(&mut self, config: TbtConfig) -> Result<(), PdError> {
        self.controller.lock().await.set_tbt_config(self.port, config).await
    }

    async fn set_usb_control(&mut self, config: UsbControlConfig) -> Result<(), PdError> {
        self.controller.lock().await.set_usb_control(self.port, config).await
    }

    async fn hard_reset(&mut self) -> Result<(), PdError> {
        self.controller.lock().await.hard_reset(self.port).await
    }

    async fn get_discovered_svids(&mut self) -> Result<DiscoveredSvids, PdError> {
        self.controller.lock().await.get_discovered_svids(self.port).await
    }

    async fn get_discover_identity_sop_response(&mut self) -> Result<sop::ResponseVdos, PdError> {
        self.controller
            .lock()
            .await
            .get_discover_identity_sop_response(self.port)
            .await
    }

    async fn get_discover_identity_sop_prime_response(&mut self) -> Result<sop_prime::ResponseVdos, PdError> {
        self.controller
            .lock()
            .await
            .get_discover_identity_sop_prime_response(self.port)
            .await
    }
}

impl<
    'device,
    C: Lockable<Inner: Pd + StateMachine>,
    Shared: Lockable<Inner = SharedState>,
    TypeCSender: NonBlockingSender<type_c_interface::service::event::PortEventData>,
    PowerSender: NonBlockingSender<power_policy_interface::psu::event::EventData>,
    LoopbackSender: NonBlockingSender<event::Loopback>,
> type_c_interface::port::pd::StateMachine for Port<'device, C, Shared, TypeCSender, PowerSender, LoopbackSender>
{
    async fn set_pd_state_machine_config(&mut self, config: PdStateMachineConfig) -> Result<(), PdError> {
        self.controller
            .lock()
            .await
            .set_pd_state_machine_config(self.port, config)
            .await
    }
}
