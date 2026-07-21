//! Max sink voltage port trait implementation
use embedded_services::{event::NonBlockingSender, sync::Lockable};
use embedded_usb_pd::PdError;
use power_policy_interface::capability::ConsumerDisconnect;
use type_c_interface::controller::max_sink_voltage::MaxSinkVoltage;

use super::*;
use crate::controller::state::SharedState;

impl<
    'device,
    C: Lockable<Inner: Pd + MaxSinkVoltage>,
    Shared: Lockable<Inner = SharedState>,
    TypeCSender: NonBlockingSender<type_c_interface::service::event::PortEventData>,
    PowerNotifier: power_policy_interface::psu::notification::Notifier,
    LoopbackSender: NonBlockingSender<event::Loopback>,
> type_c_interface::port::max_sink_voltage::MaxSinkVoltage
    for Port<'device, C, Shared, TypeCSender, PowerNotifier, LoopbackSender>
{
    async fn set_max_sink_voltage(&mut self, voltage_mv: Option<u16>) -> Result<(), PdError> {
        // A change in the maximum sink voltage can trigger a PD renegotiation. During that transition the
        // source may briefly output a voltage that does not match the active contract, which can cause an
        // overcurrent/overvoltage condition on the sink path. If we currently have a connected consumer and
        // the limit is actually changing (or being removed), disable the sink path and notify the power
        // policy that we have disconnected before applying the new limit. The power policy re-enables the
        // sink path when it reconnects the consumer to the renegotiated contract.
        let disable_sink_path = match self.psu_state.psu_state {
            PsuState::ConnectedConsumer(capability) => {
                voltage_mv.is_none() || voltage_mv != Some(capability.capability.voltage_mv)
            }
            _ => false,
        };

        if disable_sink_path {
            debug!("({}): Disabling sink path before max sink voltage change", self.name);
            self.controller.lock().await.enable_sink_path(self.port, false).await?;

            // Move our local state out of the consumer state and notify the power policy so it stops
            // tracking us as the active consumer and broadcasts a ConsumerDisconnected event. The
            // renegotiation flag marks this as a temporary disconnect for a recontract.
            if let Err(e) = self.psu_state.disconnect(true) {
                error!("({}): Error updating PSU state on disconnect: {:?}", self.name, e);
            }

            if let Err(e) = self
                .power_policy_notifier
                .notify_disconnected(ConsumerDisconnect::none().with_renegotiation(true))
                .await
            {
                error!("({}): Failed to notify power policy of disconnect: {:#?}", self.name, e);
            }
        }

        self.controller
            .lock()
            .await
            .set_max_sink_voltage(self.port, voltage_mv)
            .await
    }
}
