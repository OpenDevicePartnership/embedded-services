//! Various types of state and objects required for [`crate::wrapper::ControllerWrapper`].
//!
//! TODO: update this documentation when the type-C service is refactored
//!
use core::array::from_fn;

use cfu_service::component::CfuDevice;
use embassy_sync::{blocking_mutex::raw::RawMutex, mutex::Mutex};

use embedded_cfu_protocol::protocol_definitions::ComponentId;
use embedded_services::event;

use type_c_interface::port::{ControllerId, PortRegistration, PortStatus, event::PortStatusEventBitfield};

use crate::wrapper::proxy::{PowerProxyChannel, PowerProxyDevice, PowerProxyReceiver};

/// Service registration objects
pub struct Registration<'a, M: RawMutex> {
    pub context: &'a type_c_interface::service::context::Context,
    pub pd_controller: &'a type_c_interface::port::Device<'a>,
    pub cfu_device: &'a CfuDevice,
    pub power_devices: &'a [&'a Mutex<M, PowerProxyDevice<'a>>],
}

impl<'a, M: RawMutex> Registration<'a, M> {
    pub fn num_ports(&self) -> usize {
        self.power_devices.len()
    }
}

/// Base storage
pub struct Storage<'a, const N: usize, M: RawMutex> {
    // Registration-related
    context: &'a type_c_interface::service::context::Context,
    controller_id: ControllerId,
    pd_ports: [PortRegistration; N],
    pub cfu_device: CfuDevice,
    power_proxy_channels: [PowerProxyChannel<M>; N],
}

impl<'a, const N: usize, M: RawMutex> Storage<'a, N, M> {
    pub fn new(
        context: &'a type_c_interface::service::context::Context,
        controller_id: ControllerId,
        cfu_id: ComponentId,
        pd_ports: [PortRegistration; N],
    ) -> Self {
        Self {
            context,
            controller_id,
            pd_ports,
            cfu_device: CfuDevice::new(cfu_id),
            power_proxy_channels: from_fn(|_| PowerProxyChannel::new()),
        }
    }

    /// Create intermediate storage from this storage
    pub fn try_create_intermediate<S: event::Sender<power_policy_interface::psu::event::EventData>>(
        &self,
        power_policy_init: [(&'static str, S); N],
    ) -> Option<(IntermediateStorage<'_, N, M, S>, [PowerProxyReceiver<'_>; N])> {
        IntermediateStorage::try_from_storage(self, power_policy_init)
    }
}

pub struct Port<'a, M: RawMutex, S: event::Sender<power_policy_interface::psu::event::EventData>> {
    pub proxy: Mutex<M, PowerProxyDevice<'a>>,
    pub state: Mutex<M, PortState<S>>,
}

pub struct PortState<S: event::Sender<power_policy_interface::psu::event::EventData>> {
    /// Cached port status
    pub(crate) status: PortStatus,
    /// Software status event
    pub(crate) sw_status_event: PortStatusEventBitfield,
    /// Sender to send events to the power policy service
    pub(crate) power_policy_sender: S,
}

impl<S: event::Sender<power_policy_interface::psu::event::EventData>> PortState<S> {
    pub fn new(power_policy_sender: S) -> Self {
        Self {
            status: PortStatus::default(),
            sw_status_event: PortStatusEventBitfield::default(),
            power_policy_sender,
        }
    }
}

/// Intermediate storage that holds power proxy devices
pub struct IntermediateStorage<
    'a,
    const N: usize,
    M: RawMutex,
    S: event::Sender<power_policy_interface::psu::event::EventData>,
> {
    storage: &'a Storage<'a, N, M>,
    ports: [Port<'a, M, S>; N],
}

impl<'a, const N: usize, M: RawMutex, S: event::Sender<power_policy_interface::psu::event::EventData>>
    IntermediateStorage<'a, N, M, S>
{
    fn try_from_storage(
        storage: &'a Storage<'a, N, M>,
        power_policy_init: [(&'static str, S); N],
    ) -> Option<(Self, [PowerProxyReceiver<'a>; N])> {
        let mut ports = heapless::Vec::<_, N>::new();
        let mut power_proxy_receivers = heapless::Vec::<_, N>::new();

        for (power_proxy_channel, (name, policy_sender)) in
            storage.power_proxy_channels.iter().zip(power_policy_init.into_iter())
        {
            let (device_sender, device_receiver) = power_proxy_channel.get_device_components();

            ports
                .push(Port {
                    proxy: Mutex::new(PowerProxyDevice::new(name, device_sender, device_receiver)),
                    state: Mutex::new(PortState::new(policy_sender)),
                })
                .ok()?;
            power_proxy_receivers.push(power_proxy_channel.get_receiver()).ok()?;
        }

        Some((
            Self {
                storage,
                ports: ports.into_array().ok()?,
            },
            power_proxy_receivers.into_array().ok()?,
        ))
    }

    /// Create referenced storage from this intermediate storage
    pub fn try_create_referenced<'b>(&'b self) -> Option<ReferencedStorage<'b, N, M, S>>
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
pub struct ReferencedStorage<
    'a,
    const N: usize,
    M: RawMutex,
    S: event::Sender<power_policy_interface::psu::event::EventData>,
> {
    intermediate: &'a IntermediateStorage<'a, N, M, S>,
    pub pd_controller: type_c_interface::port::Device<'a>,
    power_devices: [&'a Mutex<M, PowerProxyDevice<'a>>; N],
}

impl<'a, const N: usize, M: RawMutex, S: event::Sender<power_policy_interface::psu::event::EventData>>
    ReferencedStorage<'a, N, M, S>
{
    /// Create a new referenced storage from the given intermediate storage
    fn try_from_intermediate(intermediate: &'a IntermediateStorage<'a, N, M, S>) -> Option<Self> {
        Some(Self {
            intermediate,
            pd_controller: type_c_interface::port::Device::new(
                intermediate.storage.controller_id,
                intermediate.storage.pd_ports.as_slice(),
            ),
            // Panic safety: will not panic because array length is fixed by generic argument
            #[allow(clippy::indexing_slicing)]
            power_devices: from_fn(|i| &intermediate.ports[i].proxy),
        })
    }

    /// Creates the backing, returns `None` if a backing has already been created
    pub fn create_backing<'b>(&'b self) -> Backing<'b, M, S>
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
        }
    }
}

/// Wrapper around registration and type-erased state
pub struct Backing<'a, M: RawMutex, S: event::Sender<power_policy_interface::psu::event::EventData>> {
    pub(crate) registration: Registration<'a, M>,
    pub(crate) ports: &'a [Port<'a, M, S>],
}
