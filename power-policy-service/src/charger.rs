//! Charger device struct and controller
use core::{future::Future, ops::DerefMut};

use embassy_futures::select::select;
use embassy_sync::{channel::Channel, mutex::Mutex};
use embedded_services::{GlobalRawMutex, debug, error, info, intrusive_list, trace, warn};

use crate::capability::{ConsumerPowerCapability, PowerCapability};

/// Charger controller trait that device drivers may use to integrate with internal messaging system
pub trait ChargeController: embedded_batteries_async::charger::Charger {
    /// Type of error returned by the bus
    type ChargeControllerError;

    /// Returns with pending events
    fn wait_event(&mut self) -> impl Future<Output = ChargerEvent>;
    /// Initialize charger hardware, after this returns the charger should be ready to charge
    fn init_charger(&mut self) -> impl Future<Output = Result<(), Self::ChargeControllerError>>;
    /// Returns if the charger hardware detects if a PSU is attached
    fn is_psu_attached(&mut self) -> impl Future<Output = Result<bool, Self::ChargeControllerError>>;
    /// Called after power policy attaches to a power port.
    fn attach_handler(
        &mut self,
        capability: ConsumerPowerCapability,
    ) -> impl Future<Output = Result<(), Self::ChargeControllerError>>;
    /// Called after power policy detaches from a power port, either to switch consumers,
    /// or because PSU was disconnected.
    fn detach_handler(&mut self) -> impl Future<Output = Result<(), Self::ChargeControllerError>>;
    /// Called when a charger CheckReady request (PolicyEvent::CheckReady) is sent to the power policy.
    /// Upon successful return of this method, the charger is assumed to be powered and ready to communicate,
    /// transitioning state from unpowered to powered.
    ///
    /// If the charger is powered, an Ok(()) does nothing. An Err(_) will put the charger into an
    /// unpowered state, meaning another PolicyEvent::CheckReady must be sent to re-establish communications
    /// with the charger. Upon successful return, the charger must be re-initialized by sending a
    /// `PolicyEvent::InitRequest`.
    fn is_ready(&mut self) -> impl Future<Output = Result<(), Self::ChargeControllerError>> {
        core::future::ready(Ok(()))
    }
}

/// Charger Device ID new type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct ChargerId(pub u8);

/// PSU state as determined by charger device
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum PsuState {
    /// Charger detected PSU attached
    Attached,
    /// Charger detected PSU detached
    Detached,
}

impl From<bool> for PsuState {
    fn from(value: bool) -> Self {
        match value {
            true => PsuState::Attached,
            false => PsuState::Detached,
        }
    }
}

/// Data for a device request
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum ChargerEvent {
    /// Charger finished initialization sequence
    Initialized(PsuState),
    /// PSU state changed
    PsuStateChange(PsuState),
    /// A timeout of some sort was detected
    Timeout,
    /// An error occured on the bus
    BusError,
}

/// Charger state errors
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum ChargerError {
    /// Charger received command in an invalid state
    InvalidState(State),
    /// Charger hardware timed out responding
    Timeout,
    /// Charger underlying bus error
    BusError,
}

impl From<ChargerError> for crate::device::Error {
    fn from(value: ChargerError) -> Self {
        Self::Charger(value)
    }
}

/// Data for a device request
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum PolicyEvent {
    /// Request to initialize charger hardware
    InitRequest,
    /// New power policy detected
    PolicyConfiguration(ConsumerPowerCapability),
    /// Request to check if the charger hardware is ready to receive communications.
    /// For example, if the charger is powered.
    CheckReady,
}

/// Data for a device request
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum ChargerResponseData {
    /// Command completed
    Ack,
    /// Charger Unpowered, but we are still Ok
    UnpoweredAck,
}

/// Response for charger requests from policy commands
pub type ChargerResponse = Result<ChargerResponseData, ChargerError>;

/// Current state of the charger
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum State {
    /// Device is unpowered
    Unpowered,
    /// Device is powered
    Powered(PoweredSubstate),
}

/// Powered state substates
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum PoweredSubstate {
    /// Device is initializing
    Init,
    /// PSU is attached and device can charge if desired
    PsuAttached,
    /// PSU is detached
    PsuDetached,
}

/// Current state of the charger
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct InternalState {
    /// Charger device state
    pub state: State,
    /// Current charger capability
    pub capability: Option<PowerCapability>,
}

/// Channel size for device requests
pub const CHARGER_CHANNEL_SIZE: usize = 1;

/// Device struct
pub struct Device {
    /// Intrusive list node
    node: intrusive_list::Node,
    /// Device ID
    id: ChargerId,
    /// Current state of the device
    state: Mutex<GlobalRawMutex, InternalState>,
    /// Channel for requests to the device
    commands: Channel<GlobalRawMutex, PolicyEvent, CHARGER_CHANNEL_SIZE>,
    /// Channel for responses from the device
    response: Channel<GlobalRawMutex, ChargerResponse, CHARGER_CHANNEL_SIZE>,
}

impl Device {
    /// Create a new device
    pub fn new(id: ChargerId) -> Self {
        Self {
            node: intrusive_list::Node::uninit(),
            id,
            state: Mutex::new(InternalState {
                state: State::Unpowered,
                capability: None,
            }),
            commands: Channel::new(),
            response: Channel::new(),
        }
    }

    /// Get the device ID
    pub fn id(&self) -> ChargerId {
        self.id
    }

    /// Returns the current state of the device
    pub async fn state(&self) -> InternalState {
        *self.state.lock().await
    }

    /// Set the state of the device
    pub async fn set_state(&self, new_state: InternalState) {
        let mut lock = self.state.lock().await;
        let current_state = lock.deref_mut();
        *current_state = new_state;
    }

    /// Wait for a command from policy
    pub async fn wait_command(&self) -> PolicyEvent {
        self.commands.receive().await
    }

    /// Send a command to the charger
    pub async fn send_command(&self, policy_event: PolicyEvent) {
        self.commands.send(policy_event).await
    }

    /// Send a response to the power policy
    pub async fn send_response(&self, response: ChargerResponse) {
        self.response.send(response).await
    }

    /// Send a command and wait for a response from the charger
    pub async fn execute_command(&self, policy_event: PolicyEvent) -> ChargerResponse {
        self.send_command(policy_event).await;
        self.response.receive().await
    }
}

impl intrusive_list::NodeContainer for Device {
    fn get_node(&self) -> &intrusive_list::Node {
        &self.node
    }
}

/// Trait for any container that holds a device
pub trait ChargerContainer {
    /// Get the underlying device struct
    fn get_charger(&self) -> &Device;
}

impl ChargerContainer for Device {
    fn get_charger(&self) -> &Device {
        self
    }
}

pub struct Wrapper<'a, C: ChargeController>
where
    ChargerError: From<<C as ChargeController>::ChargeControllerError>,
{
    charger_policy_state: &'a Device,
    controller: Mutex<GlobalRawMutex, C>,
}

impl<'a, C: ChargeController> Wrapper<'a, C>
where
    ChargerError: From<<C as ChargeController>::ChargeControllerError>,
{
    pub fn new(charger_policy_state: &'a Device, controller: C) -> Self {
        Self {
            charger_policy_state,
            controller: Mutex::new(controller),
        }
    }

    pub async fn get_state(&self) -> InternalState {
        self.charger_policy_state.state().await
    }

    pub async fn set_state(&self, new_state: InternalState) {
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
                                PsuState::Attached => State::Powered(PoweredSubstate::PsuAttached),
                                PsuState::Detached => State::Powered(PoweredSubstate::PsuDetached),
                            },
                            capability: state.capability,
                        })
                        .await
                    }
                    // If we are initializing, we don't care about anything else
                    _ => (),
                },
                PoweredSubstate::PsuAttached => match event {
                    ChargerEvent::PsuStateChange(PsuState::Detached) => {
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
                    ChargerEvent::PsuStateChange(PsuState::Attached) => {
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
                    Err(ChargerError::InvalidState(State::Unpowered))
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
                        Ok(ChargerResponseData::Ack)
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
                    Ok(ChargerResponseData::UnpoweredAck)
                }
                State::Powered(substate) => match substate {
                    PoweredSubstate::Init => {
                        error!("Charger detected new power policy configuration but charger is still initializing.");
                        Err(ChargerError::InvalidState(State::Powered(PoweredSubstate::Init)))
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
                                Ok(ChargerResponseData::Ack)
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
                                Ok(ChargerResponseData::Ack)
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
                            Ok(ChargerResponseData::Ack)
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
                            Ok(ChargerResponseData::Ack)
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
