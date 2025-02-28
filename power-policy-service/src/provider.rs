use embedded_services::trace;

use super::*;

/// Current system provider power state
pub enum PowerState {
    Low,
    High,
}

impl PowerPolicy {
    /// Computes the total requested power considering all current providers
    async fn compute_total_provider_power(&self, new_request: bool) -> Result<PowerState, Error> {
        let mut num_providers = if new_request { 1 } else { 0 };

        for device in self.context.devices().await {
            let device = device.data::<device::Device>().ok_or(Error::InvalidDevice)?;

            if device.is_provider().await {
                num_providers += 1;
            }
        }

        let request_low_power = num_providers * self.config.provider_low.max_power_mw();
        if request_low_power > self.config.high_power_threshold_mw {
            Ok(PowerState::High)
        } else {
            Ok(PowerState::Low)
        }
    }

    async fn update_provider_capability(&self, target_power: PowerCapability) -> Result<(), Error> {
        for device in self.context.devices().await {
            let device = device.data::<device::Device>().ok_or(Error::InvalidDevice)?;

            if let Ok(action) = self
                .context
                .try_policy_action::<action::ConnectedProvider>(device.id())
                .await
            {
                if action.power_capability().await != target_power {
                    if let Err(_) = action.connect_provider(target_power).await {
                        error!("Device{}: Failed to connect provider", device.id().0);
                        // Don't return to update other devices
                    }
                }
            }
        }

        Ok(())
    }

    /// Update the provider state of currently connected providers
    pub(super) async fn update_providers(&self, new_provider: Option<DeviceId>) -> Result<(), Error> {
        trace!("Updating providers");
        let target_power = match self.compute_total_provider_power(new_provider.is_some()).await? {
            PowerState::Low => self.config.provider_low,
            PowerState::High => self.config.provider_high,
        };

        self.update_provider_capability(target_power).await?;
        if let Some(new_provider) = new_provider {
            if let Ok(action) = self.context.try_policy_action::<action::Idle>(new_provider).await {
                action.connect_provider(target_power).await?;
            } else {
                error!("Device {}: Failed to connect provider", new_provider.0);
            }
        }

        Ok(())
    }
}
