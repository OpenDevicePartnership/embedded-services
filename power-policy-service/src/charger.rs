use core::cell::RefCell;

use embassy_futures::select::select;
use embedded_services::{
    debug, error, info,
    power::policy::charger::{
        self, ChargeController, ChargerEvent, ChargerResponse, InternalState, PolicyEvent, State,
    },
    trace, warn,
};

pub struct Wrapper<'a, C: ChargeController> {
    charger_policy_state: &'a charger::Device,
    controller: RefCell<C>,
}

impl<'a, C: ChargeController> Wrapper<'a, C> {
    pub fn new(charger_policy_state: &'a charger::Device, controller: C) -> Self {
        Self {
            charger_policy_state,
            controller: RefCell::new(controller),
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
            State::Init => match event {
                ChargerEvent::Initialized(psu_state) => {
                    self.set_state(InternalState {
                        state: match psu_state {
                            charger::PsuState::Attached => State::PsuAttached,
                            charger::PsuState::Detached => State::PsuDetached,
                        },
                        capability: state.capability,
                    })
                    .await
                }
                // If we are initializing, we don't care about anything else
                _ => (),
            },
            State::PsuAttached => match event {
                ChargerEvent::PsuStateChange(charger::PsuState::Detached) => {
                    self.set_state(InternalState {
                        state: State::PsuDetached,
                        capability: state.capability,
                    })
                    .await
                }
                ChargerEvent::Timeout => {
                    self.set_state(InternalState {
                        state: State::Init,
                        capability: None,
                    })
                    .await
                }
                _ => (),
            },
            State::PsuDetached => match event {
                ChargerEvent::PsuStateChange(charger::PsuState::Attached) => {
                    self.set_state(InternalState {
                        state: State::PsuAttached,
                        capability: state.capability,
                    })
                    .await
                }
                ChargerEvent::Timeout => {
                    self.set_state(InternalState {
                        state: State::Init,
                        capability: None,
                    })
                    .await
                }
                _ => (),
            },
        }
    }

    async fn process_policy_command(&self, controller: &mut C, event: PolicyEvent) {
        let state = self.get_state().await;
        let res: ChargerResponse = match event {
            PolicyEvent::InitRequest => {
                if state.state == State::Init {
                    info!("Charger received request to initialize.");
                } else {
                    warn!("Charger received request to initialize but it's already initialized! Reinitializing...");
                }

                if let Err(_err) = controller.init_charger().await {
                    error!("Charger failed initialzation sequence.");
                    Err(charger::ChargerError::BusError)
                } else {
                    Ok(charger::ChargerResponseData::Ack)
                }
            }
            PolicyEvent::PolicyConfiguration(power_capability) => match state.state {
                State::Init => {
                    error!("Charger detected new power policy configuration but charger is still initializing.");
                    Err(charger::ChargerError::InvalidState(state.state))
                }
                State::PsuAttached | State::PsuDetached => {
                    if power_capability.current_ma == 0 {
                        // Policy detected a detach
                        debug!("Charger detected new power policy configuration. Executing detach sequence");
                        if let Err(_err) = controller
                            .detach_handler()
                            .await
                            .inspect_err(|_| error!("Error executing charger power port detach sequence!"))
                        {
                            Err(charger::ChargerError::BusError)
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
                        if controller
                            .attach_handler(power_capability)
                            .await
                            .inspect_err(|_| error!("Error executing charger power port attach sequence!"))
                            .is_err()
                        {
                            Err(charger::ChargerError::BusError)
                        } else {
                            // Update power capability but do not change controller state.
                            // That is handled by process_controller_event().
                            // This way capability is cached even if the
                            // hardware charger device lags on changing its PSU state.
                            self.set_state(InternalState {
                                state: state.state,
                                capability: Some(power_capability),
                            })
                            .await;
                            Ok(charger::ChargerResponseData::Ack)
                        }
                    }
                }
            },
        };

        // Send response
        self.charger_policy_state.send_response(res).await;
    }

    #[allow(clippy::await_holding_refcell_ref)]
    pub async fn process(&self) {
        let mut controller = self.controller.borrow_mut();
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
