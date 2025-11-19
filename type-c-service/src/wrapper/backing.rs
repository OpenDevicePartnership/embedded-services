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
//! use type_c_service::wrapper::backing::{Storage, ReferencedStorage};
//!
//!
//! const NUM_PORTS: usize = 2;
//!
//! fn init(context: &'static embedded_services::type_c::controller::Context) {
//!    static STORAGE: StaticCell<Storage<NUM_PORTS, NoopRawMutex>> = StaticCell::new();
//!    let storage = STORAGE.init(Storage::new(
//!        context,
//!        ControllerId(0),
//!        0x0,
//!        [(GlobalPortId(0), power::policy::DeviceId(0)), (GlobalPortId(1), power::policy::DeviceId(1))],
//!    ));
//!    static REFERENCED: StaticCell<ReferencedStorage<NUM_PORTS, NoopRawMutex>> = StaticCell::new();
//!    let referenced = REFERENCED.init(storage.create_referenced());
//!    let _backing = referenced.create_backing().unwrap();
//! }
//! ```
use core::{
    array::from_fn,
    cell::{RefCell, RefMut},
};

use embassy_sync::{
    blocking_mutex::raw::RawMutex,
    pubsub::{DynImmediatePublisher, DynSubscriber, PubSubChannel},
};
use embassy_time::Instant;
use embedded_cfu_protocol::protocol_definitions::ComponentId;
use embedded_services::{
    power,
    type_c::{
        ControllerId,
        controller::PortStatus,
        event::{PortEvent, PortStatusChanged},
    },
};
use embedded_usb_pd::{GlobalPortId, ado::Ado};

use crate::{PortEventStreamer, wrapper::cfu};

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
struct InternalState<'a, const N: usize> {
    controller_state: ControllerState,
    port_states: [PortState<'a>; N],
}

impl<'a, const N: usize> InternalState<'a, N> {
    fn new<M: RawMutex>(storage: &'a Storage<N, M>) -> Self {
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
        }
    }
}

impl<'a, const N: usize> DynPortState<'a> for InternalState<'a, N> {
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
}

/// Trait to erase the generic port count argument
pub trait DynPortState<'a> {
    fn num_ports(&self) -> usize;

    fn port_states(&self) -> &[PortState<'a>];
    fn port_states_mut(&mut self) -> &mut [PortState<'a>];

    fn controller_state(&self) -> &ControllerState;
    fn controller_state_mut(&mut self) -> &mut ControllerState;
}

/// Service registration objects
pub struct Registration<'a> {
    pub context: &'a embedded_services::type_c::controller::Context,
    pub pd_controller: &'a embedded_services::type_c::controller::Device<'a>,
    pub cfu_device: &'a embedded_services::cfu::component::CfuDevice,
    pub power_devices: &'a [embedded_services::power::policy::device::Device],
}

impl<'a> Registration<'a> {
    pub fn num_ports(&self) -> usize {
        self.power_devices.len()
    }
}

/// PD alerts should be fairly uncommon, four seems like a reasonable number to start with.
const MAX_BUFFERED_PD_ALERTS: usize = 4;

/// Base storage
pub struct Storage<const N: usize, M: RawMutex> {
    // Registration-related
    context: &'static embedded_services::type_c::controller::Context,
    controller_id: ControllerId,
    pd_ports: [GlobalPortId; N],
    cfu_device: embedded_services::cfu::component::CfuDevice,
    power_devices: [embedded_services::power::policy::device::Device; N],

    // State-related
    pd_alerts: [PubSubChannel<M, Ado, MAX_BUFFERED_PD_ALERTS, 1, 0>; N],
}

impl<const N: usize, M: RawMutex> Storage<N, M> {
    pub fn new(
        context: &'static embedded_services::type_c::controller::Context,
        controller_id: ControllerId,
        cfu_id: ComponentId,
        ports: [(GlobalPortId, power::policy::DeviceId); N],
    ) -> Self {
        Self {
            context,
            controller_id,
            pd_ports: ports.map(|(port, _)| port),
            cfu_device: embedded_services::cfu::component::CfuDevice::new(cfu_id),
            power_devices: ports.map(|(_, device)| embedded_services::power::policy::device::Device::new(device)),
            pd_alerts: [const { PubSubChannel::new() }; N],
        }
    }

    /// Create referenced storage from this storage
    pub fn create_referenced(&self) -> ReferencedStorage<'_, N, M> {
        ReferencedStorage::from_storage(self)
    }
}

/// Contains any values that need to reference [`Storage`]
///
/// To simplify usage, we use interior mutability through a ref cell to avoid having to declare the state
/// completely separately.
pub struct ReferencedStorage<'a, const N: usize, M: RawMutex> {
    storage: &'a Storage<N, M>,
    state: RefCell<InternalState<'a, N>>,
    pd_controller: embedded_services::type_c::controller::Device<'a>,
}

impl<'a, const N: usize, M: RawMutex> ReferencedStorage<'a, N, M> {
    /// Create a new referenced storage from the given storage and controller ID
    fn from_storage(storage: &'a Storage<N, M>) -> Self {
        Self {
            storage,
            state: RefCell::new(InternalState::new(storage)),
            pd_controller: embedded_services::type_c::controller::Device::new(
                storage.controller_id,
                storage.pd_ports.as_slice(),
            ),
        }
    }

    /// Creates the backing, returns `None` if a backing has already been created
    pub fn create_backing<'b>(&'b self) -> Option<Backing<'b>>
    where
        'b: 'a,
    {
        self.state.try_borrow_mut().ok().map(|state| Backing {
            registration: Registration {
                context: self.storage.context,
                pd_controller: &self.pd_controller,
                cfu_device: &self.storage.cfu_device,
                power_devices: &self.storage.power_devices,
            },
            state,
        })
    }
}

/// Wrapper around registration and type-erased state
pub struct Backing<'a> {
    pub(crate) registration: Registration<'a>,
    pub(crate) state: RefMut<'a, dyn DynPortState<'a>>,
}
