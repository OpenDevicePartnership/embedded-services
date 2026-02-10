//! This file implements logic to determine how much power to provide to each connected device.
//! When total provided power is below [limited_power_threshold_mw](super::Config::limited_power_threshold_mw)
//! the system is in unlimited power state. In this mode up to [provider_unlimited](super::Config::provider_unlimited)
//! is provided to each device. Above this threshold, the system is in limited power state.
//! In this mode [provider_limited](super::Config::provider_limited) is provided to each device
use embedded_services::error;
use embedded_services::{debug, event::Receiver, trace};

use crate::psu;

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

impl<D: Lockable + 'static, R: Receiver<RequestData> + 'static> Service<'_, D, R>
where
    D::Inner: Psu,
{
    /// Attempt to connect the requester as a provider
    pub(super) async fn connect_provider(&self, requester_id: DeviceId) -> Result<(), Error> {
        trace!("Device{}: Attempting to connect as provider", requester_id.0);
        let requester = self.context.get_psu(requester_id)?;
        let requested_power_capability = match requester.requested_provider_capability().await {
            Some(cap) => cap,
            // Requester is no longer requesting power
            _ => {
                info!("Device{}: No-longer requesting power", requester.id().0);
                return Ok(());
            }
        };
        let mut policy_state = self.state.lock().await;
        let mut total_power_mw = 0;

        // Determine total requested power draw
        for psu in self
            .context
            .psu_devices()
            .iter_only::<psu::RegistrationEntry<'_, D, R>>()
        {
            let target_provider_cap = if psu.id() == requester_id {
                // Use the requester's requested power capability
                // this handles both new connections and upgrade requests
                Some(requested_power_capability)
            } else {
                // Use the device's current working provider capability
                psu.provider_capability().await
            };
            total_power_mw += target_provider_cap.map_or(0, |cap| cap.capability.max_power_mw());
        }

        if total_power_mw > self.config.limited_power_threshold_mw {
            policy_state.current_provider_state.state = PowerState::Limited;
        } else {
            policy_state.current_provider_state.state = PowerState::Unlimited;
        }

        debug!("New power state: {:?}", policy_state.current_provider_state.state);

        let target_power = match policy_state.current_provider_state.state {
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

        let psu = self.context.get_psu(requester_id)?;
        let mut locked_state = psu.state.lock().await;
        let mut locked_device = psu.device.lock().await;

        if let e @ Err(_) = locked_state.connect_provider(target_power) {
            error!(
                "Device{}: Cannot provide, device is in state {:#?}",
                psu.id().0,
                locked_state.state()
            );
            e
        } else {
            locked_device.connect_provider(target_power).await?;
            self.post_provider_connected(&mut policy_state, requester_id, target_power)
                .await;
            Ok(())
        }
    }

    /// Common logic for after a provider has successfully connected
    async fn post_provider_connected(
        &self,
        state: &mut InternalState,
        provider_id: DeviceId,
        target_power: ProviderPowerCapability,
    ) {
        let _ = state.connected_providers.insert(provider_id);
        self.comms_notify(CommsMessage {
            data: CommsData::ProviderConnected(provider_id, target_power),
        })
        .await;
    }
}
