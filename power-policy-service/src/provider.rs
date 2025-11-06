//! This file implements logic to determine how much power to provide to each connected device.
//! When total provided power is below [limited_power_threshold_mw](super::Config::limited_power_threshold_mw)
//! the system is in unlimited power state. In this mode up to [provider_unlimited](super::Config::provider_unlimited)
//! is provided to each device. Above this threshold, the system is in limited power state.
//! In this mode [provider_limited](super::Config::provider_limited) is provided to each device
use embedded_services::{
    debug,
    event::Receiver,
    power::policy::{device::StateKind, policy::RequestData},
    trace,
};

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

impl<D: Lockable + 'static, R: Receiver<RequestData> + 'static> PowerPolicy<D, R>
where
    D::Inner: DeviceTrait,
{
    /// Attempt to connect the requester as a provider
    pub(super) async fn connect_provider(&self, requester_id: DeviceId) -> Result<(), Error> {
        trace!("Device{}: Attempting to connect as provider", requester_id.0);
        let requester = self.context.get_device(requester_id).await?;
        let requested_power_capability = match requester.requested_provider_capability().await {
            Some(cap) => cap,
            // Requester is no longer requesting power
            _ => {
                error!("Device{}: No-longer requesting power", requester.id().0);
                return Err(Error::CannotProvide(None));
            }
        };
        let mut state = self.state.lock().await;
        let mut total_power_mw = 0;

        // Determine total requested power draw
        for device in self.context.devices().await.iter_only::<device::Device<D, R>>() {
            let target_provider_cap = if device.id() == requester_id {
                // Use the requester's requested power capability
                // this handles both new connections and upgrade requests
                Some(requested_power_capability)
            } else {
                // Use the device's current working provider capability
                device.provider_capability().await
            };
            total_power_mw += target_provider_cap.map_or(0, |cap| cap.capability.max_power_mw());

            if total_power_mw > self.config.limited_power_threshold_mw {
                state.current_provider_state.state = PowerState::Limited;
            } else {
                state.current_provider_state.state = PowerState::Unlimited;
            }
        }

        debug!("New power state: {:?}", state.current_provider_state.state);

        let target_power = match state.current_provider_state.state {
            PowerState::Limited => ProviderPowerCapability {
                capability: self.config.provider_limited,
                flags: requested_power_capability.flags,
            },
            PowerState::Unlimited => {
                if requested_power_capability.capability.max_power_mw() < self.config.provider_unlimited.max_power_mw()
                {
                    // Don't auto upgrade to a higher contract
                    requested_power_capability
                } else {
                    ProviderPowerCapability {
                        capability: self.config.provider_unlimited,
                        flags: requested_power_capability.flags,
                    }
                }
            }
        };

        let device = self.context.get_device(requester_id).await?;
        let state = device.state.lock().await.state();
        if matches!(state, device::State::Idle | device::State::ConnectedProvider(_)) {
            device.device.lock().await.connect_provider(target_power).await
        } else {
            error!(
                "Device{}: Cannot provide, device is in state {:#?}",
                device.id().0,
                state
            );
            Err(Error::InvalidState(&[StateKind::Idle], state.kind()))
        }
    }
}
