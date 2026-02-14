//! This file implements logic to determine how much power to provide to each connected device.
//! When total provided power is below [limited_power_threshold_mw](super::Config::limited_power_threshold_mw)
//! the system is in unlimited power state. In this mode up to [provider_unlimited](super::Config::provider_unlimited)
//! is provided to each device. Above this threshold, the system is in limited power state.
//! In this mode [provider_limited](super::Config::provider_limited) is provided to each device
use core::ptr;

use embedded_services::debug;
use embedded_services::error;

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

impl<'a, PSU: Lockable> Service<'a, PSU>
where
    PSU::Inner: Psu,
{
    /// Attempt to connect the requester as a provider
    pub(super) async fn connect_provider(&mut self, requester: &'a PSU) -> Result<(), Error> {
        let requested_power_capability = {
            let mut requester = requester.lock().await;
            debug!("({}): Attempting to connect as provider", requester.name());
            match requester.state().requested_provider_capability {
                Some(cap) => cap,
                // Requester is no longer requesting power
                _ => {
                    info!("({}): No-longer requesting power", requester.name());
                    return Ok(());
                }
            }
        };

        // Determine total requested power draw
        let mut total_power_mw = 0;
        for psu in self.psu_devices.iter() {
            let target_provider_cap = if ptr::eq(*psu, requester) {
                // Use the requester's requested power capability
                // this handles both new connections and upgrade requests
                Some(requested_power_capability)
            } else {
                // Use the device's current working provider capability
                psu.lock().await.state().requested_provider_capability
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

        let mut locked_requester = requester.lock().await;
        if let e @ Err(_) = locked_requester.state().connect_provider(target_power) {
            error!(
                "({}): Cannot provide, device is in state {:#?}",
                locked_requester.name(),
                locked_requester.state().psu_state
            );
            e
        } else {
            locked_requester.connect_provider(target_power).await?;
            self.post_provider_connected(requester, target_power).await;
            Ok(())
        }
    }

    /// Common logic for after a provider has successfully connected
    async fn post_provider_connected(&mut self, requester: &'a PSU, target_power: ProviderPowerCapability) {
        let _ = self.state.connected_providers.insert(requester as *const PSU);
        self.broadcast_event(ServiceEvent::ProviderConnected(requester, target_power))
            .await;
    }
}
