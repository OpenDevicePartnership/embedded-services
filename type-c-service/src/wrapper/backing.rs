//! Various types of state and objects required for [`crate::wrapper::ControllerWrapper`].
//!
//! The wrapper needs per-port state which ultimately needs to come from something like an array.
//! We need to erase the generic `N` parameter from that storage so as not to monomorphize the entire
//! wrapper over it. This module provides the necessary types and traits to do so. Things required by
//! the wrapper can be split into two categories: objects used for service registration (which must be immutable),
//! and mutable state. These are represented by the [`Registration`] and [`DynPortState`] respectively. The later
//! is a trait intended to be used as a trait object to erase the generic port count.
//!
//! [`Storage`] is the base storage type and is generic over the number of ports. However, there are additional
//! objects that need to reference the storage. To avoid a self-referential
//! struct, [`ReferencedStorage`] contains these. This struct is still generic over the number of ports.
//!
//! Lastly, [`Backing`] contains references to the registration and type-erased state and is what is passed
//! to the wrapper.
//!
//! Example usage:
//! ```
//! use embassy_sync::blocking_mutex::raw::NoopRawMutex;
//! use static_cell::StaticCell;
//! use embedded_services::type_c::ControllerId;
//! use embedded_services::power;
//! use embedded_usb_pd::GlobalPortId;
//! use type_c_service::wrapper::backing::{Storage, IntermediateStorage, ReferencedStorage};
//!
//!
//! const NUM_PORTS: usize = 2;
//!
//! fn init() {
//!    static STORAGE: StaticCell<Storage<NUM_PORTS, NoopRawMutex>> = StaticCell::new();
//!    let storage = STORAGE.init(Storage::new(
//!        ControllerId(0),
//!        0x0,
//!        [(GlobalPortId(0), power::policy::DeviceId(0)), (GlobalPortId(1), power::policy::DeviceId(1))],
//!    ));
//!    static INTERMEDIATE: StaticCell<IntermediateStorage<NUM_PORTS, NoopRawMutex>> = StaticCell::new();
//!    let intermediate = INTERMEDIATE.init(storage.create_intermediate());
//!    static REFERENCED: StaticCell<ReferencedStorage<NUM_PORTS, NoopRawMutex>> = StaticCell::new();
//!    let referenced = REFERENCED.init(intermediate.create_referenced());
//!    let _backing = referenced.create_backing().unwrap();
//! }
//! ```
use core::{
    array::from_fn,
    cell::{RefCell, RefMut},
};

use embassy_sync::{
    blocking_mutex::raw::RawMutex,
    mutex::Mutex,
    pubsub::{DynImmediatePublisher, DynSubscriber, PubSubChannel},
};
use embassy_time::Instant;
use embedded_cfu_protocol::protocol_definitions::ComponentId;
use embedded_services::{
    event,
    power::{
        self,
        policy::{DeviceId, policy},
    },
    type_c::{
        ControllerId,
        controller::PortStatus,
        event::{PortEvent, PortStatusChanged},
    },
};
use embedded_usb_pd::{GlobalPortId, ado::Ado};

use crate::{
    PortEventStreamer,
    wrapper::{
        cfu,
        proxy::{PowerProxyChannel, PowerProxyDevice, PowerProxyReceiver},
    },
};

/// Per-port state
pub struct PortState<'a> {
    /// Cached port status
    pub(crate) status: PortStatus,
    /// Software status event
    pub(crate) sw_status_event: PortStatusChanged,
    /// Sink ready deadline instant
    pub(crate) sink_ready_deadline: Option<Instant>,
    /// Pending events for the type-C service
    pub(crate) pending_events: PortEvent,
    /// PD alert channel for this port
    // There's no direct immediate equivalent of a channel. PubSubChannel has immediate publisher behavior
    // so we use that, but this requires us to keep separate publisher and subscriber objects.
    pub(crate) pd_alerts: (DynImmediatePublisher<'a, Ado>, DynSubscriber<'a, Ado>),
}

/// Internal per-controller state
#[derive(Copy, Clone)]
pub struct ControllerState {
    /// If we're currently doing a firmware update
    pub(crate) fw_update_state: cfu::FwUpdateState,
    /// State used to keep track of where we are as we turn the event bitfields into a stream of events
    pub(crate) port_event_streaming_state: Option<PortEventStreamer>,
}

impl Default for ControllerState {
    fn default() -> Self {
        Self {
            fw_update_state: cfu::FwUpdateState::Idle,
            port_event_streaming_state: None,
        }
    }
}

/// Internal state containing all per-port and per-controller state
struct InternalState<'a, const N: usize, S: event::Sender<policy::RequestData>> {
    controller_state: ControllerState,
    port_states: [PortState<'a>; N],
    port_power: [PortPower<S>; N],
}

impl<'a, const N: usize, S: event::Sender<policy::RequestData>> InternalState<'a, N, S> {
    fn new<M: RawMutex>(storage: &'a Storage<N, M>, power_events: [S; N]) -> Self {
        Self {
            controller_state: ControllerState::default(),
            port_states: from_fn(|i| PortState {
                status: PortStatus::new(),
                sw_status_event: PortStatusChanged::none(),
                sink_ready_deadline: None,
                pending_events: PortEvent::none(),
                pd_alerts: (
                    storage.pd_alerts[i].dyn_immediate_publisher(),
                    storage.pd_alerts[i].dyn_subscriber().unwrap(),
                ),
            }),
            port_power: power_events.map(|sender| PortPower {
                sender,
                state: Default::default(),
            }),
        }
    }
}

impl<'a, const N: usize, S: event::Sender<policy::RequestData>> DynPortState<'a, S> for InternalState<'a, N, S> {
    fn num_ports(&self) -> usize {
        self.port_states.len()
    }

    fn port_states(&self) -> &[PortState<'a>] {
        &self.port_states
    }

    fn port_states_mut(&mut self) -> &mut [PortState<'a>] {
        &mut self.port_states
    }

    fn controller_state(&self) -> &ControllerState {
        &self.controller_state
    }

    fn controller_state_mut(&mut self) -> &mut ControllerState {
        &mut self.controller_state
    }

    fn port_power(&self) -> &[PortPower<S>] {
        &self.port_power
    }

    fn port_power_mut(&mut self) -> &mut [PortPower<S>] {
        &mut self.port_power
    }
}

/// Trait to erase the generic port count argument
pub trait DynPortState<'a, S: event::Sender<policy::RequestData>> {
    fn num_ports(&self) -> usize;

    fn port_states(&self) -> &[PortState<'a>];
    fn port_states_mut(&mut self) -> &mut [PortState<'a>];

    fn controller_state(&self) -> &ControllerState;
    fn controller_state_mut(&mut self) -> &mut ControllerState;

    fn port_power(&self) -> &[PortPower<S>];
    fn port_power_mut(&mut self) -> &mut [PortPower<S>];
}

/// Service registration objects
pub struct Registration<'a, M: RawMutex, R: event::Receiver<policy::RequestData>> {
    pub pd_controller: &'a embedded_services::type_c::controller::Device<'a>,
    pub cfu_device: &'a embedded_services::cfu::component::CfuDevice,
    pub power_devices: &'a [embedded_services::power::policy::device::Device<'a, Mutex<M, PowerProxyDevice<'a>>, R>],
}

impl<'a, M: RawMutex, R: event::Receiver<policy::RequestData>> Registration<'a, M, R> {
    pub fn num_ports(&self) -> usize {
        self.power_devices.len()
    }
}

/// PD alerts should be fairly uncommon, four seems like a reasonable number to start with.
const MAX_BUFFERED_PD_ALERTS: usize = 4;

pub struct PortPower<S: event::Sender<policy::RequestData>> {
    pub sender: S,
    pub state: power::policy::device::InternalState,
}

/// Base storage
pub struct Storage<const N: usize, M: RawMutex> {
    // Registration-related
    controller_id: ControllerId,
    pd_ports: [GlobalPortId; N],
    cfu_device: embedded_services::cfu::component::CfuDevice,
    power_proxy_channels: [PowerProxyChannel<M>; N],

    // State-related
    pd_alerts: [PubSubChannel<M, Ado, MAX_BUFFERED_PD_ALERTS, 1, 0>; N],
}

impl<const N: usize, M: RawMutex> Storage<N, M> {
    pub fn new(controller_id: ControllerId, cfu_id: ComponentId, pd_ports: [GlobalPortId; N]) -> Self {
        Self {
            controller_id,
            pd_ports,
            cfu_device: embedded_services::cfu::component::CfuDevice::new(cfu_id),
            power_proxy_channels: from_fn(|_| PowerProxyChannel::new()),
            pd_alerts: [const { PubSubChannel::new() }; N],
        }
    }

    /// Create intermediate storage from this storage
    pub fn create_intermediate(&self) -> IntermediateStorage<'_, N, M> {
        IntermediateStorage::from_storage(self)
    }
}

/// Intermediate storage that holds power proxy devices
pub struct IntermediateStorage<'a, const N: usize, M: RawMutex> {
    storage: &'a Storage<N, M>,
    power_proxy_devices: [Mutex<M, PowerProxyDevice<'a>>; N],
    power_proxy_receivers: [Mutex<M, PowerProxyReceiver<'a>>; N],
}

impl<'a, const N: usize, M: RawMutex> IntermediateStorage<'a, N, M> {
    fn from_storage(storage: &'a Storage<N, M>) -> Self {
        let mut power_proxy_devices = heapless::Vec::<_, N>::new();
        let mut power_proxy_receivers = heapless::Vec::<_, N>::new();

        for power_proxy_channel in storage.power_proxy_channels.iter() {
            // Safe because everything has a length of N
            power_proxy_devices
                .push(Mutex::new(power_proxy_channel.get_device()))
                .unwrap_or_else(|_| panic!("Failed to insert power proxy device"));
            power_proxy_receivers
                .push(Mutex::new(power_proxy_channel.get_receiver()))
                .unwrap_or_else(|_| panic!("Failed to insert power proxy receiver"));
        }

        Self {
            storage,
            // Safe because both have N elements
            power_proxy_devices: power_proxy_devices
                .into_array()
                .unwrap_or_else(|_| panic!("Failed to create power devices")),
            power_proxy_receivers: power_proxy_receivers
                .into_array()
                .unwrap_or_else(|_| panic!("Failed to create power receivers")),
        }
    }

    /// Create referenced storage from this intermediate storage
    pub fn create_referenced<'b, S: event::Sender<policy::RequestData>, R: event::Receiver<policy::RequestData>>(
        &'b self,
        policy_args: [(DeviceId, S, R); N],
    ) -> ReferencedStorage<'b, N, M, S, R>
    where
        'b: 'a,
    {
        ReferencedStorage::from_intermediate(self, policy_args)
    }
}

/// Contains any values that need to reference [`Storage`]
///
/// To simplify usage, we use interior mutability through a ref cell to avoid having to declare the state
/// completely separately.
pub struct ReferencedStorage<
    'a,
    const N: usize,
    M: RawMutex,
    S: event::Sender<policy::RequestData>,
    R: event::Receiver<policy::RequestData>,
> {
    intermediate: &'a IntermediateStorage<'a, N, M>,
    state: RefCell<InternalState<'a, N, S>>,
    pd_controller: embedded_services::type_c::controller::Device<'a>,
    power_devices: [embedded_services::power::policy::device::Device<'a, Mutex<M, PowerProxyDevice<'a>>, R>; N],
}

impl<'a, const N: usize, M: RawMutex, S: event::Sender<policy::RequestData>, R: event::Receiver<policy::RequestData>>
    ReferencedStorage<'a, N, M, S, R>
{
    /// Create a new referenced storage from the given intermediate storage
    fn from_intermediate(intermediate: &'a IntermediateStorage<'a, N, M>, policy_args: [(DeviceId, S, R); N]) -> Self {
        let mut power_senders = heapless::Vec::<_, N>::new();
        let mut power_devices = heapless::Vec::<_, N>::new();

        for (i, (device_id, policy_sender, policy_receiver)) in policy_args.into_iter().enumerate() {
            power_senders
                .push(policy_sender)
                .unwrap_or_else(|_| panic!("Failed to insert policy sender"));
            power_devices
                .push(embedded_services::power::policy::device::Device::new(
                    device_id,
                    &intermediate.power_proxy_devices[i],
                    policy_receiver,
                ))
                .unwrap_or_else(|_| panic!("Failed to insert power device"));
        }

        Self {
            intermediate,
            state: RefCell::new(InternalState::new(
                intermediate.storage,
                // Safe because both have N elements
                power_senders
                    .into_array()
                    .unwrap_or_else(|_| panic!("Failed to create power events")),
            )),
            pd_controller: embedded_services::type_c::controller::Device::new(
                intermediate.storage.controller_id,
                intermediate.storage.pd_ports.as_slice(),
            ),
            power_devices: power_devices
                .into_array()
                .unwrap_or_else(|_| panic!("Failed to create power devices")),
        }
    }

    /// Creates the backing, returns `None` if a backing has already been created
    pub fn create_backing<'b>(&'b self) -> Option<Backing<'b, M, S, R>>
    where
        'b: 'a,
    {
        self.state.try_borrow_mut().ok().map(|state| Backing::<M, S, R> {
            registration: Registration {
                pd_controller: &self.pd_controller,
                cfu_device: &self.intermediate.storage.cfu_device,
                power_devices: &self.power_devices,
            },
            state,
            power_receivers: &self.intermediate.power_proxy_receivers,
        })
    }
}

/// Wrapper around registration and type-erased state
pub struct Backing<'a, M: RawMutex, S: event::Sender<policy::RequestData>, R: event::Receiver<policy::RequestData>> {
    pub(crate) registration: Registration<'a, M, R>,
    pub(crate) state: RefMut<'a, dyn DynPortState<'a, S>>,
    pub(crate) power_receivers: &'a [Mutex<M, PowerProxyReceiver<'a>>],
}
