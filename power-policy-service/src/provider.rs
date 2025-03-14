//! This file implements logic to determine how much power to provide to each connected device.
//! When total provided power is below [limited_power_threshold_mw](super::Config::limited_power_threshold_mw)
//! the system is in unlimited power state. In this mode [provider_unlimited](super::Config::provider_unlimited)
//! is provided to each device. Above this threshold, the system is in limited power state.
//! In this mode [provider_limited](super::Config::provider_limited) is provided to each device
//! Lastly, the system can be in recovery mode. This mode is only entered when a connected provider fails to
//! connect at a new power level. In this mode, all connected providers are set to
//! [provider_recovery](super::Config::provider_recovery). While in this mode
//! [attempt_provider_recovery](PowerPolicy::attempt_provider_recovery) is called periodically
//! which attempts to disconnect all providers in recovery mode. If this succeeds, the system will
//! return to normal operating mode.
use embedded_services::{debug, trace, warn};

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
    async fn compute_total_provider_power(&self, new_request: bool) -> PowerState {
        let mut num_providers = if new_request { 1 } else { 0 };

        for device in self.context.devices().await {
            let device = device.data::<device::Device>();
            if device.is_none() {
                // A non-power device somehow got into the list of devices, note it and move on
                warn!("Found non-power device in devices list");
                continue;
            }

            let device = device.unwrap();
            if device.is_in_recovery().await {
                // If any device is in recovery mode, we need to recover
                info!("Device {}: In recovery mode", device.id().0);
                return PowerState::Recovery;
            }

            if device.is_provider().await {
                num_providers += 1;
            }
        }

        let total_provided_power = num_providers * self.config.provider_unlimited.max_power_mw();
        if total_provided_power > self.config.limited_power_threshold_mw {
            PowerState::Limited
        } else {
            PowerState::Unlimited
        }
    }

    /// Update the power capability of all connected providers
    /// Returns true if we need to enter recovery mode
    async fn update_provider_capability(&self, target_power: PowerCapability, exit_on_recovery: bool) -> bool {
        let mut recovery = false;
        for device in self.context.devices().await {
            let device = device.data::<device::Device>();
            if device.is_none() {
                // A non-power device somehow got into the list of devices, note it and move on
                warn!("Found non-power device in devices list");
                continue;
            }

            let device = device.unwrap();
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
                            error!("Device{}: Failed to disconnect provider", device.id().0);

                            // Early exit if that's what we want
                            // This is used to avoid excessively switching power capabilities in the recovery flow
                            if exit_on_recovery {
                                return true;
                            }

                            recovery = true;
                        }
                    }
                }
            }
        }

        return recovery;
    }

    /// Update the provider state of currently connected providers
    pub(super) async fn update_providers(&self, new_provider: Option<DeviceId>) {
        trace!("Updating providers");
        let mut state = self.state.lock().await;
        let mut already_in_recovery = true;

        if state.current_provider_state.state != PowerState::Recovery {
            // Only update the power state if we're not in recovery mode
            already_in_recovery = false;
            state.current_provider_state.state = self.compute_total_provider_power(new_provider.is_some()).await;
        }
        debug!("New power state: {:?}", state.current_provider_state.state);

        let target_power = match state.current_provider_state.state {
            PowerState::Recovery => self.config.provider_recovery,
            PowerState::Unlimited => self.config.provider_unlimited,
            PowerState::Limited => self.config.provider_limited,
        };

        let recovery = self.update_provider_capability(target_power, true).await;
        if let Some(new_provider) = new_provider {
            info!("Connecting new provider");
            let connected = if let Ok(action) = self.context.try_policy_action::<action::Idle>(new_provider).await {
                let target_power = if recovery {
                    // We entered recovery mode so attempt to connect at the recovery power
                    self.config.provider_recovery
                } else {
                    target_power
                };
                action.connect_provider(target_power).await.is_ok()
            } else {
                false
            };

            // Don't enter recovery mode if we can't connect the new provider.
            // Since it's a new provider that hasn't been connected then it's
            // not drawing power as far as we're concerned
            if !connected {
                error!("Device {}: Failed to connect provider", new_provider.0);
            }
        }

        if recovery && !already_in_recovery {
            // Entering recovery, set power capability on all responding providers to recovery limit
            // Don't check return value of update_provider_capability here, if we've spontaneously recovered
            // then it'll get caught by the next call of attempt_provider_recovery
            info!("Entering recovery mode");
            let _ = self
                .update_provider_capability(self.config.provider_recovery, false)
                .await;
            state.current_provider_state.state = PowerState::Recovery;
        }
    }

    /// Wait for the next provider recovery attempt, returns true if we should call `attempt_provider_recovery`
    pub(super) async fn wait_attempt_provider_recovery(&self) -> bool {
        self.recovery_ticker.borrow_mut().next().await;
        self.state.lock().await.current_provider_state.state == PowerState::Recovery
    }

    pub(super) async fn attempt_provider_recovery(&self) {
        info!("Attempting provider recovery");
        let mut recovered = true;
        // Attempt to by disconnecting all providers in recovery
        for device in self.context.devices().await {
            let device = device.data::<device::Device>().ok_or(Error::InvalidDevice);
            if device.is_err() {
                // A non-power device somehow got into the list of devices, note it and move on
                warn!("Found non-power device in devices list");
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
                        recovered = false;
                    }
                }
            }
        }

        if !recovered {
            info!("Failed to recover all providers, staying in recovery mode");
            return;
        }

        // Attempt to restart in the unlimited power state
        self.state.lock().await.current_provider_state.state = PowerState::Unlimited;
        self.update_providers(None).await;
        if self.state.lock().await.current_provider_state.state == PowerState::Recovery {
            info!("Failed to update providers, staying in recovery mode");
            return;
        }

        info!("Successfully recovered from provider recovery mode");
    }
}
