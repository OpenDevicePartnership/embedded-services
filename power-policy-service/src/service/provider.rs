//! This file implements logic to determine how much power to provide to each connected device.
//! When total provided power is below [limited_power_threshold_mw](super::Config::limited_power_threshold_mw)
//! the system is in unlimited power state. In this mode up to [provider_unlimited](super::Config::provider_unlimited)
//! is provided to each device. Above this threshold, the system is in limited power state.
//! In this mode [provider_limited](super::Config::provider_limited) is provided to each device
use embedded_services::error;
use embedded_services::{debug, trace};

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

impl<D: Lockable + 'static> Service<'_, D>
where
    D::Inner: Psu,
{
    /// Attempt to connect the requester as a provider
    pub(super) async fn connect_provider(&mut self, requester_id: DeviceId) -> Result<(), Error> {
        trace!("Device{}: Attempting to connect as provider", requester_id.0);
        let registration = self.get_psu_registration(requester_id).ok_or(Error::InvalidDevice)?;
        let mut requester = registration.device.lock().await;
        let requested_power_capability = match requester.state().requested_provider_capability {
            Some(cap) => cap,
            // Requester is no longer requesting power
            _ => {
                info!("Device{}: No-longer requesting power", requester_id.0);
                return Ok(());
            }
        };
        let mut total_power_mw = 0;

        // Determine total requested power draw
        for psu_registration in self.psu_registration.iter() {
            let target_provider_cap = if psu_registration.id() == requester_id {
                // Use the requester's requested power capability
                // this handles both new connections and upgrade requests
                Some(requested_power_capability)
            } else {
                // Use the device's current working provider capability
                psu_registration
                    .device
                    .lock()
                    .await
                    .state()
                    .requested_provider_capability
            };
            total_power_mw += target_provider_cap.map_or(0, |cap| cap.capability.max_power_mw());
        }

        if total_power_mw > self.config.limited_power_threshold_mw {
            self.state.current_provider_state.state = PowerState::Limited;
        } else {
            self.state.current_provider_state.state = PowerState::Unlimited;
        }

        debug!("New power state: {:?}", self.state.current_provider_state.state);

        let target_power = match self.state.current_provider_state.state {
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

        if let e @ Err(_) = requester.state().connect_provider(target_power) {
            error!(
                "Device{}: Cannot provide, device is in state {:#?}",
                requester_id.0,
                requester.state().psu_state
            );
            e
        } else {
            requester.connect_provider(target_power).await?;
            self.post_provider_connected(requester_id, target_power).await;
            Ok(())
        }
    }

    /// Common logic for after a provider has successfully connected
    async fn post_provider_connected(&mut self, provider_id: DeviceId, target_power: ProviderPowerCapability) {
        let _ = self.state.connected_providers.insert(provider_id);
        self.comms_notify(CommsMessage {
            data: CommsData::ProviderConnected(provider_id, target_power),
        })
        .await;
    }
}
