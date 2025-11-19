use embassy_sync::pubsub::WaitResult;
use embedded_services::power::policy as power_policy;
use embedded_services::type_c::external::get_num_ports;

use super::*;

impl<'a> Service<'a> {
    /// Wait for a power policy event
    pub(super) async fn wait_power_policy_event(&self) -> Event<'_> {
        loop {
            match self.power_policy_event_subscriber.lock().await.next_message().await {
                WaitResult::Lagged(lagged) => {
                    // Missed some messages, all we can do is log an error
                    error!("Power policy {} event(s) lagged", lagged);
                }
                WaitResult::Message(message) => match message.data {
                    power_policy::CommsData::Unconstrained(state) => {
                        return Event::PowerPolicy(PowerPolicyEvent::Unconstrained(state));
                    }
                    _ => {
                        // No other events currently implemented
                    }
                },
            }
        }
    }

    /// Set the unconstrained state for all ports
    pub(super) async fn set_unconstrained_all(
        &self,
        unconstrained: bool,
        controllers: &intrusive_list::IntrusiveList,
    ) -> Result<(), Error> {
        for port_index in 0..embedded_services::type_c::external::get_num_ports(controllers).await {
            self.context
                .set_unconstrained_power(controllers, GlobalPortId(port_index as u8), unconstrained)
                .await?;
        }
        Ok(())
    }

    /// Processed unconstrained state change
    pub(super) async fn process_unconstrained_state_change(
        &self,
        unconstrained_state: &power_policy::UnconstrainedState,
        controllers: &intrusive_list::IntrusiveList,
    ) -> Result<(), Error> {
        if unconstrained_state.unconstrained {
            let state = self.state.lock().await;

            if unconstrained_state.available > 1 {
                // There are multiple available unconstrained consumers, set all ports to unconstrained
                // TODO: determine if we need to consider if we need to consider
                // if the system would still be unconstrained if the current consumer disconnected
                // https://github.com/OpenDevicePartnership/embedded-services/issues/428
                info!("Setting all ports to unconstrained power, multiple consumers available");
                self.set_unconstrained_all(true, controllers).await?;
            } else {
                // Only one unconstrained device is present, see if that's one of our ports
                let num_ports = get_num_ports(controllers).await;
                let unconstrained_port = state
                    .port_status
                    .iter()
                    .take(num_ports)
                    .position(|status| status.available_sink_contract.is_some() && status.unconstrained_power);

                if let Some(unconstrained_index) = unconstrained_port {
                    // One of our ports is the unconstrained consumer
                    // If it switches to sourcing then the system will no longer be unconstrained
                    // So set that port to constrained and unconstrain all others
                    info!(
                        "Setting port{} to constrained, all others unconstrained",
                        unconstrained_index
                    );
                    for port_index in 0..num_ports {
                        self.context
                            .set_unconstrained_power(
                                controllers,
                                GlobalPortId(port_index as u8),
                                port_index != unconstrained_index,
                            )
                            .await?;
                    }
                } else {
                    // The system is unconstrained, but not by one of our ports
                    // So we can set all ports to unconstrained
                    info!("Setting all ports to unconstrained power");
                    self.set_unconstrained_all(true, controllers).await?;
                }
            }
        } else {
            // No longer drawing from an unconstrained source, set all ports to constrained
            info!("Setting all ports to constrained power");
            self.set_unconstrained_all(false, controllers).await?;
        }

        Ok(())
    }

    /// Process power policy events
    pub(super) async fn process_power_policy_event(
        &self,
        message: &PowerPolicyEvent,
        controllers: &intrusive_list::IntrusiveList,
    ) -> Result<(), Error> {
        match message {
            PowerPolicyEvent::Unconstrained(state) => self.process_unconstrained_state_change(state, controllers).await,
        }
    }
}
