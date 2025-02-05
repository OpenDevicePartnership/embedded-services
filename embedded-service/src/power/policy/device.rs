//! Device struct and methods
use core::ops::DerefMut;

use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::channel::Channel;
use embassy_sync::mutex::Mutex;

use super::{policy, DeviceId, Error, PowerCapability};
use crate::{info, intrusive_list, warn};

/// Current state of the attached power device
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum State {
    /// Device is attached, but is not currently sourcing or sinking power
    Attached,
    /// Device is attached and is currently sourcing power
    Source(PowerCapability),
    /// Device is attached and is currently sinking power
    Sink(PowerCapability),
    /// No device attached
    Detached,
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
pub enum ResponseData {
    /// The request was successful
    Complete,
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
    pub async fn state(&self) -> State {
        self.state.lock().await.state
    }

    /// Returns the current sink capability of the device
    pub async fn sink_capability(&self) -> Option<PowerCapability> {
        self.state.lock().await.sink_capability
    }

    /// Returns true if the device is currently sinking power
    pub async fn is_sink(&self) -> bool {
        matches!(self.state().await, State::Sink(_))
    }

    /// Sends a request to this device and returns a response
    async fn execute_device_request(&self, request: RequestData) -> Result<ResponseData, Error> {
        self.request.send(request).await;
        self.response.receive().await
    }

    /// Connect this device as a sink
    pub async fn connect_sink(&self, capability: PowerCapability) -> Result<(), Error> {
        info!("Device {} connecting sink", self.id.0);

        let _ = self
            .execute_device_request(RequestData::ConnectSink(capability))
            .await?;

        {
            let mut lock = self.state.lock().await;
            let state = lock.deref_mut();
            if state.state != State::Attached {
                warn!("Received connect sink request for device that is not attached");
            }

            state.state = State::Sink(capability);
        }
        Ok(())
    }

    /// Connect this device as a source
    pub async fn connect_source(&self, capability: PowerCapability) -> Result<(), Error> {
        info!("Device {} connecting source", self.id.0);

        let _ = self
            .execute_device_request(RequestData::ConnectSource(capability))
            .await?;

        {
            let mut lock = self.state.lock().await;
            let state = lock.deref_mut();
            if state.state != State::Attached {
                warn!("Received connect source request for device that is not attached");
            }

            state.state = State::Source(capability);
        }
        Ok(())
    }

    /// Disconnect this device
    pub async fn disconnect(&self) -> Result<(), Error> {
        info!("Device {} disconnecting", self.id.0);

        let _ = self.execute_device_request(RequestData::Disconnect).await?;

        {
            let mut lock = self.state.lock().await;
            let state = lock.deref_mut();

            if !matches!(state.state, State::Sink(_)) && !matches!(state.state, State::Source(_)) {
                warn!(
                    "Disconnect request for device that is not sourcing or sinking, state: {:?}",
                    state.state
                );
            }

            state.state = State::Attached;
        }
        Ok(())
    }

    /// Wait for a request
    pub async fn wait_request(&self) -> RequestData {
        self.request.receive().await
    }

    /// Send a response
    pub async fn send_response(&self, response: InternalResponseData) {
        self.response.send(response).await;
    }

    /// Provide access to the power policy service
    pub fn policy<'a>(&'a self) -> PolicyInterface<'a> {
        PolicyInterface(self)
    }
}

/// Struct that provides functions to send requests to the power policy from a specific device
pub struct PolicyInterface<'a>(&'a Device);

impl PolicyInterface<'_> {
    /// Notify the power policy service that this device has attached
    pub async fn notify_attached(&self) -> Result<(), Error> {
        info!("Received attach from device {}", self.0.id().0);

        {
            let mut lock = self.0.state.lock().await;
            let state = lock.deref_mut();
            if state.state != State::Detached {
                warn!("Received attach request for device that is not detached");
            }

            state.state = State::Attached;
        }

        let _ = policy::send_request(self.0.id, policy::RequestData::NotifyAttached).await?;
        Ok(())
    }

    /// Notify the power policy service of an updated sink power capability
    pub async fn notify_sink_power_capability(&self, capability: Option<PowerCapability>) -> Result<(), Error> {
        info!("Device {} sink capability updated {:#?}", self.0.id().0, capability);

        {
            let mut lock = self.0.state.lock().await;
            let state = lock.deref_mut();
            if state.state == State::Detached {
                warn!("Received sink capability for device that is not attached");
            }

            state.sink_capability = capability;
        }
        let _ = policy::send_request(self.0.id, policy::RequestData::NotifySinkCapability(capability)).await?;
        Ok(())
    }

    /// Request the given power from the power policy service
    pub async fn request_source_power_capability(&self, capability: PowerCapability) -> Result<(), Error> {
        info!("Request source from device {}, {:#?}", self.0.id.0, capability);

        {
            let mut lock = self.0.state.lock().await;
            let state = lock.deref_mut();
            if state.state != State::Attached {
                warn!("Received request source power capability for device that is not attached");
            }
        }

        let _ = policy::send_request(self.0.id, policy::RequestData::RequestSourceCapability(capability)).await?;
        Ok(())
    }

    /// Notify the power policy service that this device cannot source or sink power anymore
    pub async fn notify_disconnect(&self) -> Result<(), Error> {
        info!("Received disconnect from device {}", self.0.id.0);

        {
            let mut lock = self.0.state.lock().await;
            let state = lock.deref_mut();
            if !matches!(state.state, State::Sink(_)) && !matches!(state.state, State::Source(_)) {
                warn!("Received disconnect request for device that is not attached");
            }

            state.state = State::Attached;
        }

        let _ = policy::send_request(self.0.id, policy::RequestData::NotifyDisconnect).await?;
        Ok(())
    }

    /// Notify the power policy service that this device has detached
    pub async fn notify_detached(&self) -> Result<(), Error> {
        info!("Received detach from device {}", self.0.id.0);

        {
            let mut lock = self.0.state.lock().await;
            let state = lock.deref_mut();
            if state.state == State::Detached {
                warn!("Received detach request for device that is not attached");
            }

            state.state = State::Detached;
        }

        let _ = policy::send_request(self.0.id, policy::RequestData::NotifyDetached).await?;
        Ok(())
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
