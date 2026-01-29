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
//! const POLICY_CHANNEL_SIZE: usize = 1;
//!
//! fn init(
//!     context: &'static embedded_services::type_c::controller::Context,
//!     power_policy_context: &'static embedded_services::power::policy::policy::Context<POLICY_CHANNEL_SIZE>
//! ) {
//!    static STORAGE: StaticCell<Storage<NUM_PORTS, NoopRawMutex, POLICY_CHANNEL_SIZE>> = StaticCell::new();
//!    let storage = STORAGE.init(Storage::new(
//!        context,
//!        ControllerId(0),
//!        0x0,
//!        [(GlobalPortId(0), power::policy::DeviceId(0)), (GlobalPortId(1), power::policy::DeviceId(1))],
//!        power_policy_context
//!    ));
//!    static REFERENCED: StaticCell<ReferencedStorage<NUM_PORTS, NoopRawMutex, POLICY_CHANNEL_SIZE>> = StaticCell::new();
//!    let referenced = REFERENCED.init(storage.create_referenced().unwrap());
//!    let _backing = referenced.create_backing().unwrap();
//! }
//! ```
use core::cell::{RefCell, RefMut};

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
    fn try_new<M: RawMutex, const POLICY_CHANNEL_SIZE: usize>(
        storage: &'a Storage<N, M, POLICY_CHANNEL_SIZE>,
    ) -> Option<Self> {
        let port_states = storage.pd_alerts.each_ref().map(|pd_alert| {
            Some(PortState {
                status: PortStatus::new(),
                sw_status_event: PortStatusChanged::none(),
                sink_ready_deadline: None,
                pending_events: PortEvent::none(),
                pd_alerts: (pd_alert.dyn_immediate_publisher(), pd_alert.dyn_subscriber().ok()?),
            })
        });

        if port_states.iter().any(|s| s.is_none()) {
            return None;
        }

        Some(Self {
            controller_state: ControllerState::default(),
            // Panic safety: All array elements checked above
            #[allow(clippy::unwrap_used)]
            port_states: port_states.map(|s| s.unwrap()),
        })
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
pub struct Registration<'a, const POLICY_CHANNEL_SIZE: usize> {
    pub context: &'a embedded_services::type_c::controller::Context,
    pub pd_controller: &'a embedded_services::type_c::controller::Device<'a>,
    pub cfu_device: &'a embedded_services::cfu::component::CfuDevice,
    pub power_devices: &'a [embedded_services::power::policy::device::Device<POLICY_CHANNEL_SIZE>],
}

impl<'a, const POLICY_CHANNEL_SIZE: usize> Registration<'a, POLICY_CHANNEL_SIZE> {
    pub fn num_ports(&self) -> usize {
        self.power_devices.len()
    }
}

/// PD alerts should be fairly uncommon, four seems like a reasonable number to start with.
const MAX_BUFFERED_PD_ALERTS: usize = 4;

/// Base storage
pub struct Storage<'a, const N: usize, M: RawMutex, const POLICY_CHANNEL_SIZE: usize> {
    // Registration-related
    context: &'a embedded_services::type_c::controller::Context,
    controller_id: ControllerId,
    pd_ports: [GlobalPortId; N],
    cfu_device: embedded_services::cfu::component::CfuDevice,
    power_devices: [embedded_services::power::policy::device::Device<POLICY_CHANNEL_SIZE>; N],

    // State-related
    pd_alerts: [PubSubChannel<M, Ado, MAX_BUFFERED_PD_ALERTS, 1, 0>; N],
}

impl<'a, const N: usize, M: RawMutex, const POLICY_CHANNEL_SIZE: usize> Storage<'a, N, M, POLICY_CHANNEL_SIZE> {
    pub fn new(
        context: &'a embedded_services::type_c::controller::Context,
        controller_id: ControllerId,
        cfu_id: ComponentId,
        ports: [(GlobalPortId, power::policy::DeviceId); N],
        power_policy_context: &'static embedded_services::power::policy::policy::Context<POLICY_CHANNEL_SIZE>,
    ) -> Self {
        Self {
            context,
            controller_id,
            pd_ports: ports.map(|(port, _)| port),
            cfu_device: embedded_services::cfu::component::CfuDevice::new(cfu_id),
            power_devices: ports
                .map(|(_, device)| embedded_services::power::policy::device::Device::new(device, power_policy_context)),
            pd_alerts: [const { PubSubChannel::new() }; N],
        }
    }

    /// Create referenced storage from this storage
    pub fn create_referenced(&self) -> Option<ReferencedStorage<'_, N, M, POLICY_CHANNEL_SIZE>> {
        ReferencedStorage::try_from_storage(self)
    }
}

/// Contains any values that need to reference [`Storage`]
///
/// To simplify usage, we use interior mutability through a ref cell to avoid having to declare the state
/// completely separately.
pub struct ReferencedStorage<'a, const N: usize, M: RawMutex, const POLICY_CHANNEL_SIZE: usize> {
    storage: &'a Storage<'a, N, M, POLICY_CHANNEL_SIZE>,
    state: RefCell<InternalState<'a, N>>,
    pd_controller: embedded_services::type_c::controller::Device<'a>,
}

impl<'a, const N: usize, M: RawMutex, const POLICY_CHANNEL_SIZE: usize>
    ReferencedStorage<'a, N, M, POLICY_CHANNEL_SIZE>
{
    /// Create a new referenced storage from the given storage and controller ID
    fn try_from_storage(storage: &'a Storage<N, M, POLICY_CHANNEL_SIZE>) -> Option<Self> {
        Some(Self {
            storage,
            state: RefCell::new(InternalState::try_new(storage)?),
            pd_controller: embedded_services::type_c::controller::Device::new(
                storage.controller_id,
                storage.pd_ports.as_slice(),
            ),
        })
    }

    /// Creates the backing, returns `None` if a backing has already been created
    pub fn create_backing<'b>(&'b self) -> Option<Backing<'b, POLICY_CHANNEL_SIZE>>
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
pub struct Backing<'a, const POLICY_CHANNEL_SIZE: usize> {
    pub(crate) registration: Registration<'a, POLICY_CHANNEL_SIZE>,
    pub(crate) state: RefMut<'a, dyn DynPortState<'a>>,
}
