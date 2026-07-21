//! Max sink voltage port trait implementation
use embassy_time::Instant;
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
    PowerSender: NonBlockingSender<power_policy_interface::psu::event::EventData>,
    LoopbackSender: NonBlockingSender<event::Loopback>,
> type_c_interface::port::max_sink_voltage::MaxSinkVoltage
    for Port<'device, C, Shared, TypeCSender, PowerSender, LoopbackSender>
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

            // In general it's not possible to know if setting the max sink voltage will trigger a renegotiation
            // because the logic to select a particular contract is specific to the PD controller.
            // Enable the sink ready timeout as a recovery mechanism. If there's no renegotiation, then the timeout
            // will result in us broadcasting the existing contract back to the power policy.
            {
                let mut shared_state = self.shared_state.lock().await;
                if shared_state.sink_ready_deadline.is_none() {
                    shared_state.sink_ready_deadline =
                        Some(Instant::now() + Self::check_sink_ready_timeout_duration(self.status.epr));
                }
            }

            // Move our local state out of the consumer state and notify the power policy so it stops
            // tracking us as the active consumer and broadcasts a ConsumerDisconnected event. The
            // renegotiation flag marks this as a temporary disconnect for a recontract.
            if let Err(e) = self.psu_state.disconnect(true) {
                error!("({}): Error updating PSU state on disconnect: {:?}", self.name, e);
            }
            if self
                .power_policy_sender
                .try_send(power_policy_interface::psu::event::EventData::Disconnected(
                    ConsumerDisconnect::none().with_renegotiation(true),
                ))
                .is_none()
            {
                error!("({}): Failed to notify power policy of disconnect", self.name);
            }
        }

        self.controller
            .lock()
            .await
            .set_max_sink_voltage(self.port, voltage_mv)
            .await
    }
}
