use core::cmp::Ordering;
use embedded_services::debug;
use embedded_services::power::policy::charger::Device as ChargerDevice;
use embedded_services::power::policy::charger::PolicyEvent;
use embedded_services::power::policy::policy::check_chargers_ready;
use embedded_services::power::policy::policy::init_chargers;

use super::*;

/// State of the current consumer
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct AvailableConsumer {
    /// The ID of the currently connected consumer
    pub device_id: DeviceId,
    /// The power capability of the currently connected consumer
    pub consumer_power_capability: ConsumerPowerCapability,
}

/// Compare two consumer capabilities to determine which one is better
///
/// This is not part of the `Ord` implementation for `ConsumerPowerCapability`, because it's specific to this implementation
fn cmp_consumer_capability(a: &ConsumerPowerCapability, b: &ConsumerPowerCapability) -> Ordering {
    (a.capability, a.flags.unconstrained_power()).cmp(&(b.capability, b.flags.unconstrained_power()))
}

impl PowerPolicy {
    /// Iterate over all devices to determine what is best power port provides the highest power
    async fn find_best_consumer(&self) -> Result<Option<AvailableConsumer>, Error> {
        let mut best_consumer = None;

        for node in self.context.devices().await {
            let device = node.data::<Device>().ok_or(Error::InvalidDevice)?;

            // Update the best available consumer
            best_consumer = match (best_consumer, device.consumer_capability().await) {
                // Nothing available
                (None, None) => None,
                // No existing consumer
                (None, Some(power_capability)) => Some(AvailableConsumer {
                    device_id: device.id(),
                    consumer_power_capability: power_capability,
                }),
                // Existing consumer, no new consumer
                (Some(_), None) => best_consumer,
                // Existing consumer, new available consumer
                (Some(best), Some(available)) => {
                    if cmp_consumer_capability(&available, &best.consumer_power_capability)
                        == core::cmp::Ordering::Greater
                    {
                        Some(AvailableConsumer {
                            device_id: device.id(),
                            consumer_power_capability: available,
                        })
                    } else {
                        best_consumer
                    }
                }
            };
        }

        Ok(best_consumer)
    }

    /// Update unconstrained state and broadcast notifications if needed
    async fn update_unconstrained_state(&self, state: &mut InternalState) -> Result<(), Error> {
        // Count how many available unconstrained devices we have
        let mut unconstrained_new = UnconstrainedState::default();
        for node in self.context.devices().await {
            let device = node.data::<Device>().ok_or(Error::InvalidDevice)?;
            if let Some(capability) = device.consumer_capability().await {
                // The device is considered unconstrained if it meets the auto unconstrained power threshold
                let auto_unconstrained = self
                    .config
                    .auto_unconstrained_threshold_mw
                    .is_some_and(|threshold| capability.capability.max_power_mw() >= threshold);
                if capability.flags.unconstrained_power() || auto_unconstrained {
                    unconstrained_new.available += 1;
                }
            }
        }

        // The overall unconstrained state is true if an unconstrained consumer is currently connected
        unconstrained_new.unconstrained = state
            .current_consumer_state
            .is_some_and(|current| current.consumer_power_capability.flags.unconstrained_power());

        if unconstrained_new != state.unconstrained {
            info!("Unconstrained state changed: {:?}", unconstrained_new);
            state.unconstrained = unconstrained_new;
            self.comms_notify(CommsMessage {
                data: CommsData::Unconstrained(state.unconstrained),
            })
            .await;
        }
        Ok(())
    }

    /// Common logic to execute after a consumer is connected
    async fn post_consumer_connected(
        &self,
        state: &mut InternalState,
        connected_consumer: AvailableConsumer,
    ) -> Result<(), Error> {
        state.current_consumer_state = Some(connected_consumer);
        // todo: review the delay time
        embassy_time::Timer::after_millis(800).await;

        // If no chargers are registered, they won't receive the new power capability.
        for node in self.context.chargers().await {
            let device = node.data::<ChargerDevice>().ok_or(Error::InvalidDevice)?;
            // Chargers should be powered at this point, but in case they are not...
            if let embedded_services::power::policy::charger::ChargerResponseData::UnpoweredAck = device
                .execute_command(PolicyEvent::PolicyConfiguration(
                    connected_consumer.consumer_power_capability.capability,
                ))
                .await?
            {
                // Force charger CheckReady and InitRequest to get it into an initialized state.
                // This condition can get hit if we did not have a previous consumer and the charger is unpowered.
                info!("Charger is unpowered, forcing charger CheckReady and Init sequence");
                check_chargers_ready().await?;
                init_chargers().await?;
                device
                    .execute_command(PolicyEvent::PolicyConfiguration(
                        connected_consumer.consumer_power_capability.capability,
                    ))
                    .await?;
            }
        }
        self.comms_notify(CommsMessage {
            data: CommsData::ConsumerConnected(
                connected_consumer.device_id,
                connected_consumer.consumer_power_capability.capability,
            ),
        })
        .await;

        Ok(())
    }

    /// Disconnect all chargers
    pub(super) async fn disconnect_chargers(&self) -> Result<(), Error> {
        for node in self.context.chargers().await {
            let device = node.data::<ChargerDevice>().ok_or(Error::InvalidDevice)?;
            if let embedded_services::power::policy::charger::ChargerResponseData::UnpoweredAck = device
                .execute_command(PolicyEvent::PolicyConfiguration(PowerCapability {
                    voltage_mv: 0,
                    current_ma: 0,
                }))
                .await?
            {
                debug!("Charger is unpowered, continuing disconnect_chargers()...");
            }
        }

        Ok(())
    }

    /// Connect to a new consumer
    async fn connect_new_consumer(
        &self,
        state: &mut InternalState,
        new_consumer: AvailableConsumer,
    ) -> Result<(), Error> {
        // Handle our current consumer
        if let Some(current_consumer) = state.current_consumer_state {
            if new_consumer.device_id == current_consumer.device_id
                && new_consumer.consumer_power_capability == current_consumer.consumer_power_capability
            {
                // If the consumer is the same device, capability, and is still available, we don't need to do anything
                info!("Best consumer is the same, not switching");
                return Ok(());
            }

            state.current_consumer_state = None;
            // Disconnect the current consumer if needed
            if let Ok(consumer) = self
                .context
                .try_policy_action::<action::ConnectedConsumer>(current_consumer.device_id)
                .await
            {
                info!(
                    "Device {}, disconnecting current consumer",
                    current_consumer.device_id.0
                );
                // disconnect current consumer and set idle
                consumer.disconnect().await?;
            }

            // If no chargers are registered, they won't receive the new power capability.
            // Also, if chargers return UnpoweredAck, that means the charger isn't powered.
            // Further down this fn the power rails are enabled and thus the charger will get power,
            // so just continue execution.
            self.disconnect_chargers().await?;

            self.comms_notify(CommsMessage {
                data: CommsData::ConsumerDisconnected(current_consumer.device_id),
            })
            .await;

            // Don't update the unconstrained here because this is a transitional state
        }

        info!("Device {}, connecting new consumer", new_consumer.device_id.0);
        if let Ok(idle) = self
            .context
            .try_policy_action::<action::Idle>(new_consumer.device_id)
            .await
        {
            idle.connect_consumer(new_consumer.consumer_power_capability).await?;
            self.post_consumer_connected(state, new_consumer).await?;
        } else if let Ok(provider) = self
            .context
            .try_policy_action::<action::ConnectedProvider>(new_consumer.device_id)
            .await
        {
            provider
                .connect_consumer(new_consumer.consumer_power_capability)
                .await?;
            state.current_consumer_state = Some(new_consumer);
            self.post_consumer_connected(state, new_consumer).await?;
        } else {
            error!("Error obtaining device in idle state");
        }

        Ok(())
    }

    /// Determines and connects the best external power
    pub(super) async fn update_current_consumer(&self) -> Result<(), Error> {
        let mut guard = self.state.lock().await;
        let state = guard.deref_mut();
        info!(
            "Selecting power port, current power: {:#?}",
            state.current_consumer_state
        );

        let best_consumer = self.find_best_consumer().await?;
        info!("Best consumer: {:#?}", best_consumer);
        if best_consumer.is_none() {
            // Notify disconnect if recently detached consumer was previously attached.
            if let Some(consumer_state) = state.current_consumer_state {
                self.comms_notify(CommsMessage {
                    data: CommsData::ConsumerDisconnected(consumer_state.device_id),
                })
                .await;
            }
            // No new consumer available
            state.current_consumer_state = None;
            self.update_unconstrained_state(state).await?;
            return Ok(());
        }
        let best_consumer = best_consumer.unwrap();

        self.connect_new_consumer(state, best_consumer).await?;
        self.update_unconstrained_state(state).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const P0: PowerCapability = PowerCapability {
        voltage_mv: 5000,
        current_ma: 1000,
    };
    const P1: PowerCapability = PowerCapability {
        voltage_mv: 5000,
        current_ma: 1500,
    };

    /// Tests the [`cmp_consumer_capability`] without any flags set.
    #[test]
    fn test_cmp_consumer_capability_no_flags() {
        let p0 = P0.into();
        let p1 = P1.into();

        assert_eq!(cmp_consumer_capability(&p0, &p1), Ordering::Less);
        assert_eq!(cmp_consumer_capability(&p1, &p1), Ordering::Equal);
        assert_eq!(cmp_consumer_capability(&p1, &p0), Ordering::Greater);
    }

    /// Tests the [`cmp_consumer_capability`] with unconstrained power flag set.
    #[test]
    fn test_cmp_consumer_capability_unconstrained() {
        let p0 = P0.into();
        let p1 = P1.into();
        let p0_unconstrained = ConsumerPowerCapability {
            capability: P0,
            flags: flags::Consumer::none().with_unconstrained_power(),
        };
        let p1_unconstrained = ConsumerPowerCapability {
            capability: P1,
            flags: flags::Consumer::none().with_unconstrained_power(),
        };

        // At the same power, the unconstrained capability should take precedence
        assert_eq!(cmp_consumer_capability(&p0_unconstrained, &p0), Ordering::Greater);

        // Unconstrained should not take precedence over higher power
        assert_eq!(cmp_consumer_capability(&p1, &p0_unconstrained), Ordering::Greater);

        // Both unconstrained, should rely on power
        assert_eq!(
            cmp_consumer_capability(&p0_unconstrained, &p1_unconstrained),
            Ordering::Less
        );
    }
}
