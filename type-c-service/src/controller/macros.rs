use embedded_services::{
    event::{NonBlockingSender, Receiver},
    sync::Lockable,
};
use type_c_interface::port::event::PortEventBitfield;

use crate::controller::{event_receiver::EventReceiver, state};

pub const DEFAULT_POWER_POLICY_CHANNEL_SIZE: usize = 2;
pub const DEFAULT_TYPE_C_CHANNEL_SIZE: usize = 2;
pub const DEFAULT_LOOPBACK_CHANNEL_SIZE: usize = 1;
pub const DEFAULT_INTERRUPT_CHANNEL_SIZE: usize = 4;

/// Components returned from port creation
pub struct PortComponents<
    'a,
    Port,
    SharedState: Lockable<Inner = state::SharedState>,
    TypeCReceiver: Receiver<type_c_interface::service::event::PortEventData>,
    PowerPolicyReceveiver: Receiver<power_policy_interface::psu::event::EventData>,
    LoopbackReceiver: Receiver<crate::controller::event::Loopback>,
    InterruptReceiver: Receiver<PortEventBitfield>,
    InterruptSender: NonBlockingSender<PortEventBitfield>,
> {
    /// Port instance
    pub port: &'a Port,
    /// Type-C service event receiver
    pub type_c_receiver: TypeCReceiver,
    /// Power policy event receiver
    pub power_policy_receiver: PowerPolicyReceveiver,
    /// Port event receiver
    pub event_receiver: EventReceiver<'a, SharedState, InterruptReceiver, LoopbackReceiver>,
    /// Interrupt sender
    pub interrupt_sender: InterruptSender,
}

/// Creates a module containing all state for a controller port, based on static cells and channels.
#[macro_export]
macro_rules! define_controller_port_static_cell_channel {
    ($vis:vis, $name:ident, $mutex:ty, $controller:ty) => {
        $vis mod $name {
            use super::*;

            // We prefix all aliases with 'Inner' to avoid potential name conflicts with user code when this macro is invoked
            // Unfortunately, super::$ty is not valid syntax in a macro, so we have to pull in everything with super::*.
            /// Type alias for the power policy sender
            pub type InnerPowerPolicyNotifierType =
                ::power_policy_interface::psu::event::NonBlockingSenderNotifier<::embassy_sync::channel::DynamicSender<'static, ::power_policy_interface::psu::event::EventData>>;
            /// Type alias for the power policy receiver
            pub type InnerPowerPolicyReceiverType =
                ::embassy_sync::channel::DynamicReceiver<'static, ::power_policy_interface::psu::event::EventData>;

            /// Type alias for the type-c service event sender
            pub type InnerTypeCSenderType = ::embassy_sync::channel::DynamicSender<'static, ::type_c_interface::service::event::PortEventData>;
            /// Type alias for the type-c service event receiver
            pub type InnerTypeCReceiverType = ::embassy_sync::channel::DynamicReceiver<'static, ::type_c_interface::service::event::PortEventData>;

            /// Type alias for the loopback sender
            pub type InnerLoopbackSenderType =
                ::embassy_sync::channel::DynamicSender<'static, $crate::controller::event::Loopback>;
            /// Type alias for the loopback receiver
            pub type InnerLoopbackReceiverType =
                ::embassy_sync::channel::DynamicReceiver<'static, $crate::controller::event::Loopback>;

            /// Type alias for the interrupt sender
            pub type InnerInterruptReceiverType =
                ::embassy_sync::channel::DynamicReceiver<'static, ::type_c_interface::port::event::PortEventBitfield>;
            /// Type alias for the interrupt receiver
            pub type InnerInterruptSenderType =
                ::embassy_sync::channel::DynamicSender<'static, ::type_c_interface::port::event::PortEventBitfield>;

            /// Type alias for the shared state mutex
            pub type InnerSharedStateType =
                ::embassy_sync::mutex::Mutex<$mutex, $crate::controller::state::SharedState>;
            /// Type alias for the port
            pub type InnerPortType = ::embassy_sync::mutex::Mutex<
                $mutex,
                $crate::controller::Port<
                    'static,
                    // Controller type
                    $controller,
                    // Shared state type
                    InnerSharedStateType,
                    // Type-C service event sender type
                    InnerTypeCSenderType,
                    // Power policy event sender type
                    InnerPowerPolicyNotifierType,
                    // Loopback event sender type
                    InnerLoopbackSenderType,
                >,
            >;

            /// Channel to send events to the type-c service
            static TYPE_C_CHANNEL: ::static_cell::StaticCell<
                ::embassy_sync::channel::Channel<
                    $mutex,
                    ::type_c_interface::service::event::PortEventData,
                    { $crate::controller::macros::DEFAULT_TYPE_C_CHANNEL_SIZE },
                >,
            > = ::static_cell::StaticCell::new();
            /// Channel to send events to the power policy service
            static POWER_POLICY_CHANNEL: ::static_cell::StaticCell<
                ::embassy_sync::channel::Channel<
                    $mutex,
                    ::power_policy_interface::psu::event::EventData,
                    { $crate::controller::macros::DEFAULT_POWER_POLICY_CHANNEL_SIZE },
                >,
            > = ::static_cell::StaticCell::new();
            /// Loopback channel
            static LOOPBACK_CHANNEL: ::static_cell::StaticCell<
                ::embassy_sync::channel::Channel<
                    $mutex,
                    $crate::controller::event::Loopback,
                    { $crate::controller::macros::DEFAULT_LOOPBACK_CHANNEL_SIZE },
                >,
            > = ::static_cell::StaticCell::new();
            /// Interrupt channel
            static INTERRUPT_CHANNEL: ::static_cell::StaticCell<
                ::embassy_sync::channel::Channel<
                    $mutex,
                    ::type_c_interface::port::event::PortEventBitfield,
                    { $crate::controller::macros::DEFAULT_INTERRUPT_CHANNEL_SIZE },
                >,
            > = ::static_cell::StaticCell::new();
            /// State shared between the port and event receiver
            static SHARED_STATE: ::static_cell::StaticCell<
                ::embassy_sync::mutex::Mutex<$mutex, $crate::controller::state::SharedState>,
            > = ::static_cell::StaticCell::new();
            /// Port instance
            static PORT: ::static_cell::StaticCell<InnerPortType> = ::static_cell::StaticCell::new();

            pub fn create(
                name: &'static str,
                port: ::embedded_usb_pd::LocalPortId,
                config: $crate::controller::config::Config,
                controller: &'static $controller,
            ) -> $crate::controller::macros::PortComponents<
                'static,
                InnerPortType,
                InnerSharedStateType,
                InnerTypeCReceiverType,
                InnerPowerPolicyReceiverType,
                InnerLoopbackReceiverType,
                InnerInterruptReceiverType,
                InnerInterruptSenderType,
            > {
                let shared_state = SHARED_STATE.init(::embassy_sync::mutex::Mutex::new(
                    $crate::controller::state::SharedState::new(),
                ));

                let power_policy_channel = POWER_POLICY_CHANNEL.init(::embassy_sync::channel::Channel::new());
                let power_policy_sender = power_policy_channel.dyn_sender();
                let power_policy_receiver = power_policy_channel.dyn_receiver();

                let type_c_channel = TYPE_C_CHANNEL.init(::embassy_sync::channel::Channel::new());
                let type_c_sender = type_c_channel.dyn_sender();
                let type_c_receiver = type_c_channel.dyn_receiver();

                let loopback_channel = LOOPBACK_CHANNEL.init(::embassy_sync::channel::Channel::new());
                let loopback_sender = loopback_channel.dyn_sender();
                let loopback_receiver = loopback_channel.dyn_receiver();

                let interrupt_channel = INTERRUPT_CHANNEL.init(::embassy_sync::channel::Channel::new());
                let interrupt_sender = interrupt_channel.dyn_sender();
                let interrupt_receiver = interrupt_channel.dyn_receiver();

                let port = PORT.init(::embassy_sync::mutex::Mutex::new($crate::controller::Port::new(
                    name,
                    config,
                    port,
                    controller,
                    shared_state,
                    type_c_sender,
                    power_policy_sender.into(),
                    loopback_sender,
                )));
                let event_receiver = $crate::controller::event_receiver::EventReceiver::new(
                    shared_state,
                    interrupt_receiver,
                    loopback_receiver,
                );
                $crate::controller::macros::PortComponents {
                    port,
                    type_c_receiver,
                    power_policy_receiver,
                    event_receiver,
                    interrupt_sender,
                }
            }
        }
    };
}
