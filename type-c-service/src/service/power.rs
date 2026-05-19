use core::ptr;

use embedded_services::sync::Lockable as _;
use power_policy_interface::service as power_policy;
use power_policy_interface::service::event::EventData as PowerPolicyEventData;
use type_c_interface::port::pd::Pd as _;

use super::*;

impl<'a, Reg: Registration<'a>> Service<'a, Reg> {
    /// Set the unconstrained state for all ports
    pub(super) async fn set_unconstrained_all(&mut self, unconstrained: bool) -> Result<(), Error> {
        for port in self.registration.ports() {
            port.lock().await.set_unconstrained_power(unconstrained).await?;
        }
        Ok(())
    }

    /// Processed unconstrained state change
    pub(super) async fn process_unconstrained_state_change(
        &mut self,
        unconstrained_state: &power_policy::UnconstrainedState,
    ) -> Result<(), Error> {
        if unconstrained_state.unconstrained {
            if unconstrained_state.available > 1 {
                // There are multiple available unconstrained consumers, set all ports to unconstrained
                // TODO: determine if we need to consider if we need to consider
                // if the system would still be unconstrained if the current consumer disconnected
                // https://github.com/OpenDevicePartnership/embedded-services/issues/428
                info!("Setting all ports to unconstrained power, multiple consumers available");
                self.set_unconstrained_all(true).await?;
            } else {
                // Only one unconstrained device is present, see if that's one of our ports
                let mut unconstrained_port = None;
                for port in self.registration.ports().iter() {
                    let status = port.lock().await.get_port_status().await?;
                    if status.available_sink_contract.is_some() && status.unconstrained_power {
                        unconstrained_port = Some(*port);
                        break;
                    }
                }

                if let Some(unconstrained_port) = unconstrained_port {
                    // One of our ports is the unconstrained consumer
                    // If it switches to sourcing then the system will no longer be unconstrained
                    // So set that port to constrained and unconstrain all others
                    info!(
                        "Setting port ({}) to constrained, all others unconstrained",
                        unconstrained_port.lock().await.name()
                    );
                    for port in self.registration.ports().iter() {
                        port.lock()
                            .await
                            .set_unconstrained_power(!ptr::eq(*port, unconstrained_port))
                            .await?;
                    }
                } else {
                    // The system is unconstrained, but not by one of our ports
                    // So we can set all ports to unconstrained
                    info!("Setting all ports to unconstrained power");
                    self.set_unconstrained_all(true).await?;
                }
            }
        } else {
            // No longer drawing from an unconstrained source, set all ports to constrained
            info!("Setting all ports to constrained power");
            self.set_unconstrained_all(false).await?;
        }

        Ok(())
    }

    /// Process power policy events
    pub(super) async fn process_power_policy_event(&mut self, message: &PowerPolicyEventData) -> Result<(), Error> {
        match message {
            PowerPolicyEventData::Unconstrained(state) => self.process_unconstrained_state_change(state).await,
            PowerPolicyEventData::ConsumerDisconnected => {
                self.ucsi.psu_connected = false;
                // Notify OPM because this can affect battery charging capability status
                if self.ucsi.notifications_enabled.battery_charge_change() {
                    self.pend_ucsi_connected_ports().await;
                }
                Ok(())
            }
            PowerPolicyEventData::ConsumerConnected(_) => {
                self.ucsi.psu_connected = true;
                // Notify OPM because this can affect battery charging capability status
                if self.ucsi.notifications_enabled.battery_charge_change() {
                    self.pend_ucsi_connected_ports().await;
                }
                Ok(())
            }
            _ => Ok(()), // Other events don't require any action from the service
        }
    }
}
