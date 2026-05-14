//! Retimer port trait implementation
use embedded_services::{event::Sender, sync::Lockable};
use embedded_usb_pd::PdError;
use type_c_interface::control::retimer::RetimerFwUpdateState;
use type_c_interface::controller::retimer::Retimer;

use super::*;
use crate::controller::state::SharedState;

impl<
    'device,
    C: Lockable<Inner: Pd + Retimer>,
    Shared: Lockable<Inner = SharedState>,
    TypeCSender: Sender<type_c_interface::service::event::PortEventData>,
    PowerSender: Sender<power_policy_interface::psu::event::EventData>,
    LoopbackSender: Sender<event::Loopback>,
> type_c_interface::port::retimer::Retimer for Port<'device, C, Shared, TypeCSender, PowerSender, LoopbackSender>
{
    async fn get_rt_fw_update_status(&mut self) -> Result<RetimerFwUpdateState, PdError> {
        self.controller.lock().await.get_rt_fw_update_status(self.port).await
    }

    async fn set_rt_fw_update_state(&mut self) -> Result<(), PdError> {
        self.controller.lock().await.set_rt_fw_update_state(self.port).await
    }

    async fn clear_rt_fw_update_state(&mut self) -> Result<(), PdError> {
        self.controller.lock().await.clear_rt_fw_update_state(self.port).await
    }

    async fn set_rt_compliance(&mut self) -> Result<(), PdError> {
        self.controller.lock().await.set_rt_compliance(self.port).await
    }

    async fn reconfigure_retimer(&mut self) -> Result<(), PdError> {
        self.controller.lock().await.reconfigure_retimer(self.port).await
    }
}
