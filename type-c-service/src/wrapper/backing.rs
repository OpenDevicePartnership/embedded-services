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
//! use crate::type_c::ControllerId;
//! use embedded_services::power;
//! use embedded_usb_pd::GlobalPortId;
//! use type_c_service::wrapper::backing::{Storage, IntermediateStorage, ReferencedStorage};
//! use embassy_sync::channel::{Channel, DynamicSender};
//! use power_policy_service::psu;
//!
//! fn init(context: &'static crate::type_c::controller::Context) {
//!    static STORAGE: StaticCell<Storage<1, NoopRawMutex>> = StaticCell::new();
//!    let storage = STORAGE.init(Storage::new(
//!        context,
//!        ControllerId(0),
//!        0x0, // CFU component ID (unused)
//!        [GlobalPortId(0)],
//!    ));
//!
//!    static POLICY_CHANNEL: StaticCell<Channel<NoopRawMutex, psu::event::EventData, 1>> = StaticCell::new();
//!    let policy_channel = POLICY_CHANNEL.init(Channel::new());
//!    let policy_sender = policy_channel.dyn_sender();
//!
//!    static INTERMEDIATE: StaticCell<IntermediateStorage<1, NoopRawMutex>> =
//!        StaticCell::new();
//!    let intermediate = INTERMEDIATE.init(
//!        storage
//!            .try_create_intermediate([("Pd0", policy_sender)])
//!            .expect("Failed to create intermediate storage"),
//!    );
//!
//!    static REFERENCED: StaticCell<
//!        ReferencedStorage<1, NoopRawMutex>,
//!    > = StaticCell::new();
//!    let referenced = REFERENCED.init(
//!        intermediate
//!            .try_create_referenced()
//!            .expect("Failed to create referenced storage"),
//!    );
//! }
//! ```
use core::array::from_fn;

use cfu_service::component::CfuDevice;
use embassy_sync::{
    blocking_mutex::raw::RawMutex,
    channel::DynamicSender,
    mutex::Mutex,
    pubsub::{DynImmediatePublisher, DynSubscriber, PubSubChannel},
};

use embassy_time::Instant;
use embedded_cfu_protocol::protocol_definitions::ComponentId;
use embedded_usb_pd::{GlobalPortId, ado::Ado};

use crate::type_c::{
    ControllerId,
    controller::PortStatus,
    event::{PortEvent, PortStatusChanged},
};

use crate::{
    PortEventStreamer,
    wrapper::{
        cfu,
        proxy::{PowerProxyChannel, PowerProxyDevice, PowerProxyReceiver},
    },
};

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

/// Service registration objects
pub struct Registration<'a, M: RawMutex> {
    pub context: &'a crate::type_c::controller::Context,
    pub pd_controller: &'a crate::type_c::controller::Device<'a>,
    pub cfu_device: &'a CfuDevice,
    pub power_devices: &'a [&'a Mutex<M, PowerProxyDevice<'a>>],
}

impl<'a, M: RawMutex> Registration<'a, M> {
    pub fn num_ports(&self) -> usize {
        self.power_devices.len()
    }
}

/// PD alerts should be fairly uncommon, four seems like a reasonable number to start with.
const MAX_BUFFERED_PD_ALERTS: usize = 4;

/// Base storage
pub struct Storage<'a, const N: usize, M: RawMutex> {
    // Registration-related
    context: &'a crate::type_c::controller::Context,
    controller_id: ControllerId,
    pd_ports: [GlobalPortId; N],
    cfu_device: CfuDevice,
    power_proxy_channels: [PowerProxyChannel<M>; N],

    // State-related
    pd_alerts: [PubSubChannel<M, Ado, MAX_BUFFERED_PD_ALERTS, 1, 0>; N],
}

impl<'a, const N: usize, M: RawMutex> Storage<'a, N, M> {
    pub fn new(
        context: &'a crate::type_c::controller::Context,
        controller_id: ControllerId,
        cfu_id: ComponentId,
        pd_ports: [GlobalPortId; N],
    ) -> Self {
        Self {
            context,
            controller_id,
            pd_ports,
            cfu_device: CfuDevice::new(cfu_id),
            power_proxy_channels: from_fn(|_| PowerProxyChannel::new()),
            pd_alerts: [const { PubSubChannel::new() }; N],
        }
    }

    /// Create intermediate storage from this storage
    pub fn try_create_intermediate(
        &'a self,
        power_policy_init: [(
            &'static str,
            DynamicSender<'a, power_policy_service::psu::event::EventData>,
        ); N],
    ) -> Option<IntermediateStorage<'a, N, M>> {
        IntermediateStorage::try_from_storage(self, power_policy_init)
    }
}

pub struct Port<'a, M: RawMutex> {
    pub proxy: Mutex<M, PowerProxyDevice<'a>>,
    pub state: Mutex<M, PortState<'a>>,
}

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
    /// Sender to send events to the power policy service
    pub(crate) power_policy_sender: DynamicSender<'a, power_policy_service::psu::event::EventData>,
}

impl<'a> PortState<'a> {
    pub fn new(
        pd_alerts: (DynImmediatePublisher<'a, Ado>, DynSubscriber<'a, Ado>),
        power_policy_sender: DynamicSender<'a, power_policy_service::psu::event::EventData>,
    ) -> Self {
        Self {
            status: PortStatus::default(),
            sw_status_event: PortStatusChanged::default(),
            sink_ready_deadline: None,
            pending_events: PortEvent::default(),
            pd_alerts,
            power_policy_sender,
        }
    }
}

/// Intermediate storage that holds power proxy devices
pub struct IntermediateStorage<'a, const N: usize, M: RawMutex> {
    storage: &'a Storage<'a, N, M>,
    ports: [Port<'a, M>; N],
    power_proxy_receivers: [Mutex<M, PowerProxyReceiver<'a>>; N],
}

impl<'a, const N: usize, M: RawMutex> IntermediateStorage<'a, N, M> {
    fn try_from_storage(
        storage: &'a Storage<'a, N, M>,
        power_policy_init: [(
            &'static str,
            DynamicSender<'a, power_policy_service::psu::event::EventData>,
        ); N],
    ) -> Option<Self> {
        let mut ports = heapless::Vec::<_, N>::new();
        let mut power_proxy_receivers = heapless::Vec::<_, N>::new();

        for ((power_proxy_channel, pd_alert), (name, policy_sender)) in storage
            .power_proxy_channels
            .iter()
            .zip(storage.pd_alerts.iter())
            .zip(power_policy_init.into_iter())
        {
            let (device_sender, device_receiver) = power_proxy_channel.get_device_components();

            ports
                .push(Port {
                    proxy: Mutex::new(PowerProxyDevice::new(name, device_sender, device_receiver)),
                    state: Mutex::new(PortState::new(
                        (pd_alert.dyn_immediate_publisher(), pd_alert.dyn_subscriber().ok()?),
                        policy_sender,
                    )),
                })
                .ok()?;
            power_proxy_receivers
                .push(Mutex::new(power_proxy_channel.get_receiver()))
                .ok()?;
        }

        Some(Self {
            storage,
            ports: ports.into_array().ok()?,
            power_proxy_receivers: power_proxy_receivers.into_array().ok()?,
        })
    }

    /// Create referenced storage from this intermediate storage
    pub fn try_create_referenced<'b>(&'b self) -> Option<ReferencedStorage<'b, N, M>>
    where
        'b: 'a,
    {
        ReferencedStorage::try_from_intermediate(self)
    }
}

/// Contains any values that need to reference [`Storage`]
///
/// To simplify usage, we use interior mutability through a ref cell to avoid having to declare the state
/// completely separately.
pub struct ReferencedStorage<'a, const N: usize, M: RawMutex> {
    intermediate: &'a IntermediateStorage<'a, N, M>,
    pd_controller: crate::type_c::controller::Device<'a>,
    power_devices: [&'a Mutex<M, PowerProxyDevice<'a>>; N],
}

impl<'a, const N: usize, M: RawMutex> ReferencedStorage<'a, N, M> {
    /// Create a new referenced storage from the given intermediate storage
    fn try_from_intermediate(intermediate: &'a IntermediateStorage<'a, N, M>) -> Option<Self> {
        Some(Self {
            intermediate,
            pd_controller: crate::type_c::controller::Device::new(
                intermediate.storage.controller_id,
                intermediate.storage.pd_ports.as_slice(),
            ),
            power_devices: from_fn(|i| &intermediate.ports[i].proxy),
        })
    }

    /// Creates the backing, returns `None` if a backing has already been created
    pub fn create_backing<'b>(&'b self) -> Backing<'b, M>
    where
        'b: 'a,
    {
        Backing {
            registration: Registration {
                context: self.intermediate.storage.context,
                pd_controller: &self.pd_controller,
                cfu_device: &self.intermediate.storage.cfu_device,
                power_devices: &self.power_devices,
            },
            ports: &self.intermediate.ports,
            power_receivers: &self.intermediate.power_proxy_receivers,
        }
    }
}

/// Wrapper around registration and type-erased state
pub struct Backing<'a, M: RawMutex> {
    pub(crate) registration: Registration<'a, M>,
    pub(crate) ports: &'a [Port<'a, M>],
    pub(crate) power_receivers: &'a [Mutex<M, PowerProxyReceiver<'a>>],
}
