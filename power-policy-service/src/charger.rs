use embassy_sync::mutex::Mutex;
use embedded_services::GlobalRawMutex;

use embassy_futures::select::select;
use embedded_services::{
    debug, error, info,
    power::policy::charger::{
        self, ChargeController, ChargerEvent, ChargerResponse, InternalState, PolicyEvent, PoweredSubstate, State,
    },
    trace, warn,
};

pub struct Wrapper<'a, C: ChargeController>
where
    charger::ChargerError: From<<C as ChargeController>::ChargeControllerError>,
{
    charger_policy_state: &'a charger::Device,
    controller: Mutex<GlobalRawMutex, C>,
}

impl<'a, C: ChargeController> Wrapper<'a, C>
where
    charger::ChargerError: From<<C as ChargeController>::ChargeControllerError>,
{
    pub fn new(charger_policy_state: &'a charger::Device, controller: C) -> Self {
        Self {
            charger_policy_state,
            controller: Mutex::new(controller),
        }
    }

    pub async fn get_state(&self) -> charger::InternalState {
        self.charger_policy_state.state().await
    }

    pub async fn set_state(&self, new_state: charger::InternalState) {
        self.charger_policy_state.set_state(new_state).await
    }

    async fn wait_policy_command(&self) -> PolicyEvent {
        self.charger_policy_state.wait_command().await
    }

    #[allow(clippy::single_match)]
    async fn process_controller_event(&self, _controller: &mut C, event: ChargerEvent) {
        let state = self.get_state().await;
        match state.state {
            State::Powered(powered_substate) => match powered_substate {
                PoweredSubstate::Init => match event {
                    ChargerEvent::Initialized(psu_state) => {
                        self.set_state(InternalState {
                            state: match psu_state {
                                charger::PsuState::Attached => State::Powered(PoweredSubstate::PsuAttached),
                                charger::PsuState::Detached => State::Powered(PoweredSubstate::PsuDetached),
                            },
                            capability: state.capability,
                        })
                        .await
                    }
                    // If we are initializing, we don't care about anything else
                    _ => (),
                },
                PoweredSubstate::PsuAttached => match event {
                    ChargerEvent::PsuStateChange(charger::PsuState::Detached) => {
                        self.set_state(InternalState {
                            state: State::Powered(PoweredSubstate::PsuDetached),
                            capability: state.capability,
                        })
                        .await
                    }
                    ChargerEvent::Timeout => {
                        self.set_state(InternalState {
                            state: State::Powered(PoweredSubstate::Init),
                            capability: None,
                        })
                        .await
                    }
                    _ => (),
                },
                PoweredSubstate::PsuDetached => match event {
                    ChargerEvent::PsuStateChange(charger::PsuState::Attached) => {
                        self.set_state(InternalState {
                            state: State::Powered(PoweredSubstate::PsuAttached),
                            capability: state.capability,
                        })
                        .await
                    }
                    ChargerEvent::Timeout => {
                        self.set_state(InternalState {
                            state: State::Powered(PoweredSubstate::Init),
                            capability: None,
                        })
                        .await
                    }
                    _ => (),
                },
            },
            State::Unpowered => warn!(
                "Charger is unpowered but ChargeController event received event: {:?}",
                event
            ),
        }
    }

    async fn process_policy_command(&self, controller: &mut C, event: PolicyEvent) {
        let state = self.get_state().await;
        let res: ChargerResponse = match event {
            PolicyEvent::InitRequest => {
                if state.state == State::Unpowered {
                    error!("Charger received request to initialize but it's unpowered!");
                    Err(charger::ChargerError::InvalidState(State::Unpowered))
                } else {
                    if state.state == State::Powered(PoweredSubstate::Init) {
                        info!("Charger received request to initialize.");
                    } else {
                        warn!("Charger received request to initialize but it's already initialized! Reinitializing...");
                    }

                    if let Err(err) = controller.init_charger().await {
                        error!("Charger failed initialzation sequence.");
                        Err(err.into())
                    } else {
                        Ok(charger::ChargerResponseData::Ack)
                    }
                }
            }
            PolicyEvent::PolicyConfiguration(power_capability) => match state.state {
                State::Unpowered => {
                    // Power policy sends this event when a new type-c plug event comes in
                    // For the scenario where the charger is unpowered, we don't want to block the power policy
                    // from completing it's connect_consumer() call, as there might be cases where we don't want
                    // chargers to be powered or the charger can't be powered.
                    error!("Charger detected new power policy configuration but it's unpowered!");
                    Ok(charger::ChargerResponseData::UnpoweredAck)
                }
                State::Powered(substate) => match substate {
                    PoweredSubstate::Init => {
                        error!("Charger detected new power policy configuration but charger is still initializing.");
                        Err(charger::ChargerError::InvalidState(State::Powered(
                            PoweredSubstate::Init,
                        )))
                    }
                    PoweredSubstate::PsuAttached | PoweredSubstate::PsuDetached => {
                        if power_capability.capability.current_ma == 0 {
                            // Policy detected a detach
                            debug!("Charger detected new power policy configuration. Executing detach sequence");
                            if let Err(err) = controller
                                .detach_handler()
                                .await
                                .inspect_err(|_| error!("Error executing charger power port detach sequence!"))
                            {
                                Err(err.into())
                            } else {
                                // Update power capability but do not change controller state.
                                // That is handled by process_controller_event().
                                // This way capability is cached even if the
                                // hardware charger device lags on changing its PSU state.
                                self.set_state(InternalState {
                                    state: state.state,
                                    capability: None,
                                })
                                .await;
                                Ok(charger::ChargerResponseData::Ack)
                            }
                        } else {
                            // Policy detected an attach
                            debug!("Charger detected new power policy configuration. Executing attach sequence");
                            if let Err(err) = controller
                                .attach_handler(power_capability)
                                .await
                                .inspect_err(|_| error!("Error executing charger power port attach sequence!"))
                            {
                                Err(err.into())
                            } else {
                                // Update power capability but do not change controller state.
                                // That is handled by process_controller_event().
                                // This way capability is cached even if the
                                // hardware charger device lags on changing its PSU state.
                                self.set_state(InternalState {
                                    state: state.state,
                                    capability: Some(power_capability.capability),
                                })
                                .await;
                                Ok(charger::ChargerResponseData::Ack)
                            }
                        }
                    }
                },
            },
            PolicyEvent::CheckReady => {
                debug!("Charger received check ready request.");
                let ret = controller.is_ready().await;
                match state.state {
                    State::Powered(_) => {
                        if let Err(e) = ret {
                            self.set_state(InternalState {
                                state: State::Unpowered,
                                // Cache capability for logging/debug
                                capability: state.capability,
                            })
                            .await;
                            Err(e.into())
                        } else {
                            Ok(charger::ChargerResponseData::Ack)
                        }
                    }
                    State::Unpowered => {
                        if let Err(e) = ret {
                            Err(e.into())
                        } else {
                            self.set_state(InternalState {
                                state: State::Powered(PoweredSubstate::Init),
                                capability: None,
                            })
                            .await;
                            Ok(charger::ChargerResponseData::Ack)
                        }
                    }
                }
            }
        };

        // Send response
        self.charger_policy_state.send_response(res).await;
    }

    pub async fn process(&self) {
        let mut controller = self.controller.lock().await;
        loop {
            let res = select(controller.wait_event(), self.wait_policy_command()).await;
            match res {
                embassy_futures::select::Either::First(event) => {
                    trace!("New charger device event.");
                    self.process_controller_event(&mut controller, event).await;
                }
                embassy_futures::select::Either::Second(event) => {
                    trace!("New charger policy command.");
                    self.process_policy_command(&mut controller, event).await;
                }
            };
        }
    }
}
