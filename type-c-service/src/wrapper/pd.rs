use crate::wrapper::event_receiver::SinkReadyTimeoutEvent;
use embassy_time::Duration;
use embedded_services::debug;
use embedded_usb_pd::constants::{T_PS_TRANSITION_EPR_MS, T_PS_TRANSITION_SPR_MS};

use super::*;

impl<
    'device,
    M: RawMutex,
    D: Lockable,
    S: event::Sender<power_policy_interface::psu::event::EventData>,
    V: FwOfferValidator,
> ControllerWrapper<'device, M, D, S, V>
where
    D::Inner: Controller,
{
    /// Check the sink ready timeout
    ///
    /// After accepting a sink contract (new contract as consumer), the PD spec guarantees that the
    /// source will be available to provide power after `tPSTransition`. This allows us to handle transitions
    /// even for controllers that might not always broadcast sink ready events.
    pub(super) fn check_sink_ready_timeout<const N: usize>(
        &self,
        sink_ready_timeout: &mut SinkReadyTimeoutEvent<N>,
        previous_status: &PortStatus,
        new_status: &PortStatus,
        port: LocalPortId,
        new_contract: bool,
        sink_ready: bool,
    ) -> Result<(), PdError> {
        let contract_changed = previous_status.available_sink_contract != new_status.available_sink_contract;
        let timeout = sink_ready_timeout.get_timeout(port);

        // Don't start the timeout if the sink has signaled it's ready or if the contract didn't change.
        // The latter ensures that soft resets won't continually reset the ready timeout
        debug!(
            "Port{}: Check sink ready: new_contract={:?}, sink_ready={:?}, contract_changed={:?}, deadline={:?}",
            port.0, new_contract, sink_ready, contract_changed, timeout,
        );
        if new_contract && !sink_ready && contract_changed {
            // Start the timeout
            // Double the spec maximum transition time to provide a safety margin for hardware/controller delays or out-of-spec controllers.
            let timeout_ms = if new_status.epr {
                T_PS_TRANSITION_EPR_MS
            } else {
                T_PS_TRANSITION_SPR_MS
            }
            .maximum
            .0 * 2;

            debug!("Port{}: Sink ready timeout started for {}ms", port.0, timeout_ms);
            sink_ready_timeout.set_timeout(port, Instant::now() + Duration::from_millis(timeout_ms as u64));
        } else if timeout.is_some()
            && (!new_status.is_connected() || new_status.available_sink_contract.is_none() || sink_ready)
        {
            debug!("Port{}: Sink ready timeout cleared", port.0);
            sink_ready_timeout.clear_timeout(port);
        }
        Ok(())
    }
}
