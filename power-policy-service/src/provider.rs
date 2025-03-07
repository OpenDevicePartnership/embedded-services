//! This file implements logic to determine how much power to provide to each connected device.
//! When total provided power is below [limited_power_threshold_mw](super::Config::limited_power_threshold_mw)
//! the system is in unlimited power state. In this mode [provider_unlimited](super::Config::provider_unlimited)
//! is provided to each device. Above this threshold, the system is in limited power state.
//! In this mode [provider_limited](super::Config::provider_limited) is provided to each device
use embedded_services::{debug, trace};

use super::*;

/// Current system provider power state
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum PowerState {
    /// Recovery mode, system is in the process of recovering from a fault on one or more ports
    Recovery,
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
    /// Computes the total requested power considering all current providers
    async fn compute_total_provider_power(&self, new_request: bool) -> Result<PowerState, Error> {
        let mut num_providers = if new_request { 1 } else { 0 };

        for device in self.context.devices().await {
            let device = device.data::<device::Device>().ok_or(Error::InvalidDevice)?;

            if device.is_in_recovery().await {
                // If any device is in recovery mode, we need to recover
                info!("Device {}: In recovery mode", device.id().0);
                return Ok(PowerState::Recovery);
            }

            if device.is_provider().await {
                num_providers += 1;
            }
        }

        let total_provided_power = num_providers * self.config.provider_unlimited.max_power_mw();
        if total_provided_power > self.config.limited_power_threshold_mw {
            Ok(PowerState::Limited)
        } else {
            Ok(PowerState::Unlimited)
        }
    }

    /// Update the power capability of all connected providers
    /// Returns true if we need to enter recovery mode
    async fn update_provider_capability(&self, target_power: PowerCapability) -> Result<bool, Error> {
        let mut recovery = false;
        for device in self.context.devices().await {
            let device = device.data::<device::Device>().ok_or(Error::InvalidDevice)?;

            if let Ok(action) = self
                .context
                .try_policy_action::<action::ConnectedProvider>(device.id())
                .await
            {
                if action.power_capability().await != target_power {
                    // Attempt to connect at new capability. Don't exit early if this fails so
                    // we can continue to attempt to connect other providers
                    if let Err(_) = action.connect_provider(target_power).await {
                        error!(
                            "Device{}: Failed to connect provider, attempting to disconnect",
                            device.id().0
                        );

                        if let Err(_) = action.disconnect().await {
                            error!(
                                "Device{}: Failed to disconnect provider, entering recovery mode",
                                device.id().0
                            );
                            recovery = true;
                        }
                    }
                }
            }
        }
        Ok(recovery)
    }

    /// Update the provider state of currently connected providers
    pub(super) async fn update_providers(&self, new_provider: Option<DeviceId>) -> Result<(), Error> {
        trace!("Updating providers");
        let mut state = self.state.lock().await;

        if state.current_provider_state.state != PowerState::Recovery {
            // Only update the power state if we're not in recovery mode
            state.current_provider_state.state = self.compute_total_provider_power(new_provider.is_some()).await?;
        }
        debug!("New power state: {:?}", state.current_provider_state.state);

        let target_power = match state.current_provider_state.state {
            PowerState::Recovery => self.config.provider_recovery,
            PowerState::Unlimited => self.config.provider_unlimited,
            PowerState::Limited => self.config.provider_limited,
        };

        let recovery = self.update_provider_capability(target_power).await?;
        if let Some(new_provider) = new_provider {
            info!("Connecting new provider");
            if let Ok(action) = self.context.try_policy_action::<action::Idle>(new_provider).await {
                action.connect_provider(target_power).await?;
            } else {
                // Don't enter recovery mode if we can't connect to the new provider
                // Since it's a new provider that hasn't been connected then it's
                // not drawing power as far as we're concerned
                error!("Device {}: Failed to connect provider", new_provider.0);
            }
        }

        if recovery {
            state.current_provider_state.state = PowerState::Recovery;
        }

        Ok(())
    }

    pub(super) async fn attempt_provider_recovery(&self) {
        self.recovery_ticker.borrow_mut().next().await;
        if self.state.lock().await.current_provider_state.state != PowerState::Recovery {
            // Not in recovery mode
            return;
        }

        info!("Attempting provider recovery");
        // Attempt to by disconnecting all providers in recovery
        for device in self.context.devices().await {
            let device = device.data::<device::Device>().ok_or(Error::InvalidDevice);
            if device.is_err() {
                continue;
            }

            let device = device.unwrap();
            if device.is_in_recovery().await {
                if let Ok(action) = self
                    .context
                    .try_policy_action::<action::ConnectedProvider>(device.id())
                    .await
                {
                    if let Err(_) = action.disconnect().await {
                        error!("Device {}: Failed to recover", device.id().0);
                    }
                }
            }
        }

        // Attempt to restart in the limited power state
        self.state.lock().await.current_provider_state.state = PowerState::Limited;
        if self.update_providers(None).await.is_err() {
            // Failed to update providers, stay in recovery mode
            info!("Failed to update providers, staying in recovery mode");
            return;
        }

        if self.state.lock().await.current_provider_state.state != PowerState::Recovery {
            // Successfully recovered
            info!("Successfully recovered from provider recovery mode");
        } else {
            // Still in recovery mode
            info!("Still in provider recovery mode");
        }
    }
}
