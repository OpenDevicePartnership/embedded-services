use core::cmp::Ordering;
use core::ops::DerefMut;
use embedded_services::{debug, error};

use super::*;

use crate::capability::ConsumerFlags;
use crate::charger::Device as ChargerDevice;
use crate::{capability::ConsumerPowerCapability, charger::PolicyEvent, device::State};

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
/// This is not part of the `Ord` implementation for `ConsumerPowerCapability`, because it's specific to this implementation.
/// *_is_current indicate if the device with that capability is the currently connected consumer. This is used to make the
/// implementation stick so as to avoid switching between otherwise equivalent consumers.
fn cmp_consumer_capability(
    a: &ConsumerPowerCapability,
    a_is_current: bool,
    b: &ConsumerPowerCapability,
    b_is_current: bool,
) -> Ordering {
    (a.capability, a_is_current).cmp(&(b.capability, b_is_current))
}

impl<D: Lockable + 'static, R: Receiver<RequestData> + 'static> Service<'_, D, R>
where
    D::Inner: DeviceTrait,
{
    /// Iterate over all devices to determine what is best power port provides the highest power
    async fn find_best_consumer(&self, state: &InternalState) -> Result<Option<AvailableConsumer>, Error> {
        let mut best_consumer = None;
        let current_consumer_id = state.current_consumer_state.map(|f| f.device_id);

        for node in self.context.devices() {
            let device = node.data::<Device<'_, D, R>>().ok_or(Error::InvalidDevice)?;

            let consumer_capability = device.consumer_capability().await;
            // Don't consider consumers below minimum threshold
            if consumer_capability
                .zip(self.config.min_consumer_threshold_mw)
                .is_some_and(|(cap, min)| cap.capability.max_power_mw() < min)
            {
                info!(
                    "Device{}: Not considering consumer, power capability is too low",
                    device.id().0,
                );
                continue;
            }

            // Update the best available consumer
            best_consumer = match (best_consumer, consumer_capability) {
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
                    if cmp_consumer_capability(
                        &available,
                        Some(device.id()) == current_consumer_id,
                        &best.consumer_power_capability,
                        Some(best.device_id) == current_consumer_id,
                    ) == core::cmp::Ordering::Greater
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
        for node in self.context.devices() {
            let device = node.data::<Device<'_, D, R>>().ok_or(Error::InvalidDevice)?;
            if let Some(capability) = device.consumer_capability().await {
                if capability.flags.unconstrained_power() {
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
        for node in self.context.chargers() {
            let device = node.data::<ChargerDevice>().ok_or(Error::InvalidDevice)?;
            // Chargers should be powered at this point, but in case they are not...
            if let crate::charger::ChargerResponseData::UnpoweredAck = device
                .execute_command(PolicyEvent::PolicyConfiguration(
                    connected_consumer.consumer_power_capability,
                ))
                .await?
            {
                // Force charger CheckReady and InitRequest to get it into an initialized state.
                // This condition can get hit if we did not have a previous consumer and the charger is unpowered.
                info!("Charger is unpowered, forcing charger CheckReady and Init sequence");
                self.context.check_chargers_ready().await?;
                self.context.init_chargers().await?;
                device
                    .execute_command(PolicyEvent::PolicyConfiguration(
                        connected_consumer.consumer_power_capability,
                    ))
                    .await?;
            }
        }
        self.comms_notify(CommsMessage {
            data: CommsData::ConsumerConnected(
                connected_consumer.device_id,
                connected_consumer.consumer_power_capability,
            ),
        })
        .await;

        Ok(())
    }

    /// Disconnect all chargers
    pub(super) async fn disconnect_chargers(&self) -> Result<(), Error> {
        for node in self.context.chargers() {
            let device = node.data::<ChargerDevice>().ok_or(Error::InvalidDevice)?;
            if let crate::charger::ChargerResponseData::UnpoweredAck = device
                .execute_command(PolicyEvent::PolicyConfiguration(ConsumerPowerCapability {
                    capability: PowerCapability {
                        voltage_mv: 0,
                        current_ma: 0,
                    },
                    flags: ConsumerFlags::none(),
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
            let consumer_device = self.context.get_device(current_consumer.device_id)?;
            let mut locked_state = consumer_device.state.lock().await;
            let mut locked_device = consumer_device.device.lock().await;

            if matches!(locked_state.state(), State::ConnectedConsumer(_)) {
                // Disconnect the current consumer if needed
                info!("Device{}: Disconnecting current consumer", current_consumer.device_id.0);
                // disconnect current consumer and set idle
                locked_device.disconnect().await?;
                if let Err(e) = locked_state.disconnect(false) {
                    // This should never happen because we check the state above, log an error instead of a panic
                    error!(
                        "Device{}: Disconnect transition failed: {:#?}",
                        current_consumer.device_id.0, e
                    );
                }
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
        let device = self.context.get_device(new_consumer.device_id)?;
        let mut locked_device = device.device.lock().await;
        let mut locked_state = device.state.lock().await;

        if let e @ Err(_) = locked_state.connect_consumer(new_consumer.consumer_power_capability) {
            error!(
                "Device{}: Not ready to connect consumer, state: {:#?}",
                device.id().0,
                locked_state.state()
            );
            e
        } else {
            locked_device
                .connect_consumer(new_consumer.consumer_power_capability)
                .await?;
            self.post_consumer_connected(state, new_consumer).await
        }
    }

    /// Determines and connects the best external power
    pub(super) async fn update_current_consumer(&self) -> Result<(), Error> {
        let mut guard = self.state.lock().await;
        let state = guard.deref_mut();
        info!(
            "Selecting power port, current power: {:#?}",
            state.current_consumer_state
        );

        let best_consumer = self.find_best_consumer(state).await?;
        info!("Best consumer: {:#?}", best_consumer);
        if let Some(best_consumer) = best_consumer {
            self.connect_new_consumer(state, best_consumer).await?;
        } else {
            // Notify disconnect if recently detached consumer was previously attached.
            if let Some(consumer_state) = state.current_consumer_state {
                self.disconnect_chargers().await?;
                self.comms_notify(CommsMessage {
                    data: CommsData::ConsumerDisconnected(consumer_state.device_id),
                })
                .await;
            }
            // No new consumer available
            state.current_consumer_state = None;
        }

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

        assert_eq!(cmp_consumer_capability(&p0, false, &p1, false), Ordering::Less);
        assert_eq!(cmp_consumer_capability(&p1, false, &p1, false), Ordering::Equal);
        assert_eq!(cmp_consumer_capability(&p1, false, &p0, false), Ordering::Greater);
    }
}
