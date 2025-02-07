//! Device struct and methods
use core::ops::DerefMut;

use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::channel::Channel;
use embassy_sync::mutex::Mutex;

use super::{policy, DeviceId, Error, PowerCapability};
use crate::{info, intrusive_list};

/// Most basic device states
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum StateKind {
    /// No device attached
    Detached,
    /// Device is attached
    Attached,
    /// Device is sourcing power
    Source,
    /// Device is sinking power
    Sink,
}

/// Current state of the power device
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
enum State {
    /// Device is attached, but is not currently sourcing or sinking power
    Attached,
    /// Device is attached and is currently sourcing power
    Source(PowerCapability),
    /// Device is attached and is currently sinking power
    Sink(PowerCapability),
    /// No device attached
    Detached,
}

impl State {
    fn kind(&self) -> StateKind {
        match self {
            State::Attached => StateKind::Attached,
            State::Source(_) => StateKind::Source,
            State::Sink(_) => StateKind::Sink,
            State::Detached => StateKind::Detached,
        }
    }
}

/// Internal device state for power policy
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
struct InternalState {
    /// Current state of the device
    pub state: State,
    /// Current sink capability
    pub sink_capability: Option<PowerCapability>,
}

/// Data for a device request
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum RequestData {
    /// Start sinking on this port
    ConnectSink(PowerCapability),
    /// Start sourcing on this port
    ConnectSource(PowerCapability),
    /// Stop sourcing or sinking on this port
    Disconnect,
}

/// Request from power policy service to a device
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Request {
    /// Target device
    pub id: DeviceId,
    /// Request data
    pub data: RequestData,
}

/// Data for a device response
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum ResponseData {
    /// The request was successful
    Complete,
}

impl ResponseData {
    /// Returns an InvalidResponse error if the response is not complete
    pub fn complete_or_err(self) -> Result<(), Error> {
        match self {
            ResponseData::Complete => Ok(()),
        }
    }
}

/// Wrapper type to make code cleaner
pub type InternalResponseData = Result<ResponseData, Error>;

/// Response from a device to the power policy service
pub struct Response {
    /// Target device
    pub id: DeviceId,
    /// Response data
    pub data: ResponseData,
}

/// Channel size for device requests
pub const DEVICE_CHANNEL_SIZE: usize = 1;

/// Device struct
pub struct Device {
    /// Intrusive list node
    node: intrusive_list::Node,
    /// Device ID
    id: DeviceId,
    /// Current state of the device
    state: Mutex<NoopRawMutex, InternalState>,
    /// Channel for requests to the device
    request: Channel<NoopRawMutex, RequestData, DEVICE_CHANNEL_SIZE>,
    /// Channel for responses from the device
    response: Channel<NoopRawMutex, InternalResponseData, DEVICE_CHANNEL_SIZE>,
}

impl Device {
    /// Create a new device
    pub fn new(id: DeviceId) -> Self {
        Self {
            node: intrusive_list::Node::uninit(),
            id,
            state: Mutex::new(InternalState {
                state: State::Detached,
                sink_capability: None,
            }),
            request: Channel::new(),
            response: Channel::new(),
        }
    }

    /// Get the device ID
    pub fn id(&self) -> DeviceId {
        self.id
    }

    /// Returns the current state of the device
    async fn state(&self) -> State {
        self.state.lock().await.state
    }

    /// Returns the current sink capability of the device
    pub async fn sink_capability(&self) -> Option<PowerCapability> {
        self.state.lock().await.sink_capability
    }

    /// Returns true if the device is currently sinking power
    pub async fn is_sink(&self) -> bool {
        self.state().await.kind() == StateKind::Sink
    }

    /// Sends a request to this device and returns a response
    async fn execute_device_request(&self, request: RequestData) -> Result<ResponseData, Error> {
        self.request.send(request).await;
        self.response.receive().await
    }

    /// Wait for a request
    pub async fn wait_request(&self) -> RequestData {
        self.request.receive().await
    }

    /// Send a response
    pub async fn send_response(&self, response: InternalResponseData) {
        self.response.send(response).await;
    }

    async fn set_state(&self, new_state: State) {
        let mut lock = self.state.lock().await;
        let state = lock.deref_mut();
        state.state = new_state;
    }

    async fn update_sink_capability(&self, capability: Option<PowerCapability>) {
        let mut lock = self.state.lock().await;
        let state = lock.deref_mut();
        state.sink_capability = capability;
    }
    /// Try to provide access to the device-side state machine
    pub async fn try_device_state_machine<'a, S: state_machine::Kind>(
        &'a self,
    ) -> Result<state_machine::Device<'a, S>, Error> {
        let state = self.state().await.kind();
        if S::kind() != state {
            return Err(Error::InvalidState(S::kind(), state));
        }
        Ok(state_machine::Device::new(self))
    }

    /// Try to provide access to the power policy-side state machine
    pub async fn try_policy_state_machine<'a, S: state_machine::Kind>(
        &'a self,
    ) -> Result<state_machine::Policy<'a, S>, Error> {
        let state = self.state().await.kind();
        if S::kind() != state {
            return Err(Error::InvalidState(S::kind(), state));
        }
        Ok(state_machine::Policy::new(self))
    }
}

impl intrusive_list::NodeContainer for Device {
    fn get_node(&self) -> &crate::Node {
        &self.node
    }
}

/// Trait for any container that holds a device
pub trait DeviceContainer {
    /// Get the underlying device struct
    fn get_power_policy_device(&self) -> &Device;
}

impl DeviceContainer for Device {
    fn get_power_policy_device(&self) -> &Device {
        self
    }
}

/// States for compile time enforced state machine
pub mod state_machine {
    use super::*;

    /// Trait to provide the kind of a state type
    pub trait Kind {
        /// Return the kind of a state type
        fn kind() -> StateKind;
    }

    /// State machine for device that is detached
    pub struct Detached;
    impl Kind for Detached {
        fn kind() -> StateKind {
            StateKind::Detached
        }
    }

    /// State machine for device that is attached
    pub struct Attached;
    impl Kind for Attached {
        fn kind() -> StateKind {
            StateKind::Attached
        }
    }

    /// State machine for device that is sourcing power
    pub struct Source;
    impl Kind for Source {
        fn kind() -> StateKind {
            StateKind::Source
        }
    }

    /// State machine for device that is sinking power
    pub struct Sink;
    impl Kind for Sink {
        fn kind() -> StateKind {
            StateKind::Sink
        }
    }

    /// Device state machine control
    pub struct Device<'a, S: Kind> {
        device: &'a super::Device,
        _state: core::marker::PhantomData<S>,
    }

    impl<'a, S: Kind> Device<'a, S> {
        /// Create a new state machine
        pub(super) fn new(device: &'a super::Device) -> Self {
            Self {
                device,
                _state: core::marker::PhantomData,
            }
        }

        /// Detach the device
        pub async fn detach(self) -> Result<Device<'a, Detached>, Error> {
            info!("Received detach from device {}", self.device.id.0);
            self.device.set_state(State::Detached).await;
            policy::send_request(self.device.id, policy::RequestData::NotifyDetached)
                .await?
                .complete_or_err()?;
            Ok(Device::new(self.device))
        }

        /// Disconnect this device
        async fn disconnect_internal(&self) -> Result<(), Error> {
            info!("Device {} disconnecting", self.device.id.0);
            self.device.set_state(State::Attached).await;
            policy::send_request(self.device.id, policy::RequestData::NotifyDisconnect)
                .await?
                .complete_or_err()
        }
    }

    impl<'a> Device<'a, Detached> {
        /// Attach the device
        pub async fn attach(self) -> Result<Device<'a, Attached>, Error> {
            info!("Received attach from device {}", self.device.id.0);
            self.device.set_state(State::Attached).await;
            policy::send_request(self.device.id, policy::RequestData::NotifyAttached)
                .await?
                .complete_or_err()?;
            Ok(Device::new(self.device))
        }
    }

    impl<'a> Device<'a, Attached> {
        /// Notify the power policy service of an updated sink power capability
        pub async fn notify_sink_power_capability(&self, capability: Option<PowerCapability>) -> Result<(), Error> {
            info!("Device {} sink capability updated {:#?}", self.device.id.0, capability);
            self.device.update_sink_capability(capability).await;
            policy::send_request(self.device.id, policy::RequestData::NotifySinkCapability(capability))
                .await?
                .complete_or_err()
        }

        /// Request the given power from the power policy service
        pub async fn request_source_power_capability(&self, capability: PowerCapability) -> Result<(), Error> {
            info!("Request source from device {}, {:#?}", self.device.id.0, capability);
            policy::send_request(self.device.id, policy::RequestData::RequestSourceCapability(capability))
                .await?
                .complete_or_err()
        }
    }

    impl<'a> Device<'a, Sink> {
        /// Disconnect this device
        pub async fn disconnect(self) -> Result<Device<'a, Attached>, Error> {
            self.disconnect_internal().await?;
            Ok(Device::new(self.device))
        }
    }

    impl<'a> Device<'a, Source> {
        /// Disconnect this device
        pub async fn disconnect(self) -> Result<Device<'a, Attached>, Error> {
            self.disconnect_internal().await?;
            Ok(Device::new(self.device))
        }
    }

    /// Policy state machine control
    pub struct Policy<'a, S: Kind> {
        device: &'a super::Device,
        _state: core::marker::PhantomData<S>,
    }

    impl<'a, S: Kind> Policy<'a, S> {
        /// Create a new state machine
        pub(super) fn new(device: &'a super::Device) -> Self {
            Self {
                device,
                _state: core::marker::PhantomData,
            }
        }

        async fn disconnect_internal(&self) -> Result<(), Error> {
            info!("Device {} got disconnect request", self.device.id.0);
            self.device
                .execute_device_request(RequestData::Disconnect)
                .await?
                .complete_or_err()
        }
    }

    // The policy can do nothing when no device is attached
    impl Policy<'_, Detached> {}

    impl<'a> Policy<'a, Attached> {
        /// Connect this device as a sink
        pub async fn connect_sink(self, capability: PowerCapability) -> Result<Policy<'a, Sink>, Error> {
            info!("Device {} connecting sink", self.device.id.0);

            self.device
                .execute_device_request(RequestData::ConnectSink(capability))
                .await?
                .complete_or_err()?;

            self.device.set_state(State::Sink(capability)).await;
            Ok(Policy::new(self.device))
        }

        /// Connect this device as a source
        pub async fn connect_source(self, capability: PowerCapability) -> Result<Device<'a, Source>, Error> {
            info!("Device {} connecting source", self.device.id.0);

            self.device
                .execute_device_request(RequestData::ConnectSource(capability))
                .await?
                .complete_or_err()?;

            self.device.set_state(State::Source(capability)).await;
            Ok(Device::new(self.device))
        }
    }

    impl<'a> Policy<'a, Sink> {
        /// Disconnect this device
        pub async fn disconnect(self) -> Result<Policy<'a, Attached>, Error> {
            self.disconnect_internal().await?;
            Ok(Policy::new(self.device))
        }
    }

    impl<'a> Policy<'a, Source> {
        /// Disconnect this device
        pub async fn disconnect(self) -> Result<Policy<'a, Attached>, Error> {
            self.disconnect_internal().await?;
            Ok(Policy::new(self.device))
        }
    }
}
