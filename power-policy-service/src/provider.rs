//! This file implements logic to determine how much power to provide to each connected device.
//! When total provided power is below [limited_power_threshold_mw](super::Config::limited_power_threshold_mw)
//! the system is in unlimited power state. In this mode up to [provider_unlimited](super::Config::provider_unlimited)
//! is provided to each device. Above this threshold, the system is in limited power state.
//! In this mode [provider_limited](super::Config::provider_limited) is provided to each device
use embedded_services::trace;

use super::*;

/// Current system provider power state
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum PowerState {
    /// System is capable of providing high power
    #[default]
    Unlimited,
    /// System can only provide limited power
    Limited,
}

/// Power policy provider global state
#[derive(Clone, Copy, Default)]
pub(super) struct State {
    /// Current power state
    state: PowerState,
}

impl PowerPolicy {
    /// Attempt to connect the requester as a provider
    pub(super) async fn connect_provider(&self, requester_id: DeviceId) {
        trace!("Device{}: Attempting to connect as provider", requester_id.0);
        let requester = match self.context.get_device(requester_id).await {
            Ok(device) => device,
            Err(_) => {
                error!("Device{}: Invalid device", requester_id.0);
                return;
            }
        };
        let requested_power_capability = match requester.requested_provider_capability().await {
            Some(cap) => cap,
            // Requester is no longer requesting power
            _ => {
                info!("Device{}: No-longer requesting power", requester.id().0);
                return;
            }
        };

        let connected = if let Ok(action) = self.context.try_policy_action::<action::Idle>(requester.id()).await {
            let _ = action.connect_provider(requested_power_capability).await;
            Ok(())
        } else if let Ok(action) = self
            .context
            .try_policy_action::<action::ConnectedProvider>(requester.id())
            .await
        {
            let _ = action.connect_provider(requested_power_capability).await;
            Ok(())
        } else {
            Err(Error::InvalidState(
                device::StateKind::Idle,
                requester.state().await.kind(),
            ))
        };

        // Don't need to do anything special, the device is responsible for attempting to reconnect
        if let Err(e) = connected {
            error!("Device{}: Failed to connect as provider, {:#?}", requester.id().0, e);
        }
    }
}
