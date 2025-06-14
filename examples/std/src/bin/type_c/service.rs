use core::future::poll_fn;
use core::future::Future;
use core::task::Poll;
use embassy_executor::{Executor, Spawner};
use embassy_sync::once_lock::OnceLock;
use embassy_time::Timer;
use embedded_cfu_protocol::protocol_definitions::{FwUpdateOfferResponse, HostToken};
use embedded_services::comms;
use embedded_services::power::{self, policy};
use embedded_services::type_c::{controller, ControllerId};
use embedded_usb_pd::type_c::Current;
use embedded_usb_pd::type_c::Current as TypecCurrent;
use embedded_usb_pd::Error;
use embedded_usb_pd::GlobalPortId;
use embedded_usb_pd::PortId as LocalPortId;
use embedded_usb_pd::PowerRole;
use log::*;
use static_cell::StaticCell;

const CONTROLLER0: ControllerId = ControllerId(0);
const PORT0: GlobalPortId = GlobalPortId(0);
const POWER0: power::policy::DeviceId = power::policy::DeviceId(0);

mod test_controller {
    use std::cell::Cell;

    use embassy_sync::{blocking_mutex::raw::NoopRawMutex, signal::Signal};
    use embedded_services::type_c::{
        controller::{Contract, ControllerStatus, PortStatus, RetimerFwUpdateState},
        event::PortEventKind,
    };

    use super::*;

    pub struct ControllerState {
        events: Signal<NoopRawMutex, PortEventKind>,
        status: Cell<PortStatus>,
    }

    impl ControllerState {
        pub fn new() -> Self {
            Self {
                events: Signal::new(),
                status: Cell::new(PortStatus::default()),
            }
        }

        /// Simulate a connection
        pub fn connect(&self, _contract: Contract) {
            self.status.set(PortStatus::new());

            let mut events = PortEventKind::none();
            events.set_plug_inserted_or_removed(true);
            events.set_new_power_contract_as_consumer(true);
            self.events.signal(events);
        }

        /// Simulate a sink connecting
        pub fn connect_sink(&self, current: Current) {
            self.connect(Contract::Sink(current.into()));
        }

        /// Simulate a disconnection
        pub fn disconnect(&self) {
            self.status.set(PortStatus::default());

            let mut events = PortEventKind::none();
            events.set_plug_inserted_or_removed(true);
            self.events.signal(events);
        }

        /// Simulate a debug accessory source connecting
        pub fn connect_debug_accessory_source(&self, _current: Current) {
            self.status.set(PortStatus::new());

            let mut events = PortEventKind::none();
            events.set_plug_inserted_or_removed(true);
            events.set_new_power_contract_as_consumer(true);
            self.events.signal(events);
        }
    }

    pub struct Controller<'a> {
        state: &'a ControllerState,
        events: Cell<PortEventKind>,
    }

    impl<'a> Controller<'a> {
        pub fn new(state: &'a ControllerState) -> Self {
            Self {
                state,
                events: Cell::new(PortEventKind::none()),
            }
        }
    }

    impl embedded_services::type_c::controller::Controller for Controller<'_> {
        type BusError = ();

        async fn sync_state(&mut self) -> Result<(), Error<Self::BusError>> {
            Ok(())
        }

        async fn wait_port_event(&mut self) -> Result<(), Error<Self::BusError>> {
            trace!("Wait for port event");
            let events = self.state.events.wait().await;
            trace!("Port event: {:#?}", events);
            self.events.set(events);
            Ok(())
        }

        async fn clear_port_events(&mut self, _port: LocalPortId) -> Result<PortEventKind, Error<Self::BusError>> {
            let events = self.events.get();
            debug!("Clear port events: {:#?}", events);
            self.events.set(PortEventKind::none());
            Ok(events)
        }

        async fn get_port_status(&mut self, _port: LocalPortId) -> Result<PortStatus, Error<Self::BusError>> {
            debug!("Get port status: {:#?}", self.state.status.get());
            Ok(self.state.status.get())
        }

        async fn enable_sink_path(&mut self, _port: LocalPortId, enable: bool) -> Result<(), Error<Self::BusError>> {
            debug!("Enable sink path: {}", enable);
            Ok(())
        }

        async fn get_controller_status(&mut self) -> Result<ControllerStatus<'static>, Error<Self::BusError>> {
            debug!("Get controller status");
            Ok(ControllerStatus {
                mode: "Test",
                valid_fw_bank: true,
                fw_version0: 0xbadf00d,
                fw_version1: 0xdeadbeef,
            })
        }

        async fn get_rt_fw_update_status(
            &mut self,
            _port: LocalPortId,
        ) -> Result<RetimerFwUpdateState, Error<Self::BusError>> {
            debug!("Get retimer fw update status");
            Ok(RetimerFwUpdateState::Inactive)
        }

        async fn set_rt_fw_update_state(&mut self, _port: LocalPortId) -> Result<(), Error<Self::BusError>> {
            debug!("Get set fw update status");
            Ok(())
        }

        async fn clear_rt_fw_update_state(&mut self, _port: LocalPortId) -> Result<(), Error<Self::BusError>> {
            debug!("Get clear fw update status");
            Ok(())
        }

        async fn get_active_fw_version(&self) -> Result<u32, Error<Self::BusError>> {
            Ok(0)
        }

        async fn start_fw_update(&mut self) -> Result<(), Error<Self::BusError>> {
            Ok(())
        }

        async fn abort_fw_update(&mut self) -> Result<(), Error<Self::BusError>> {
            Ok(())
        }

        async fn finalize_fw_update(&mut self) -> Result<(), Error<Self::BusError>> {
            Ok(())
        }

        async fn write_fw_contents(&mut self, _offset: usize, _data: &[u8]) -> Result<(), Error<Self::BusError>> {
            Ok(())
        }
    }

    pub struct Validator;

    impl type_c_service::wrapper::FwOfferValidator for Validator {
        fn validate(
            &self,
            _current: embedded_cfu_protocol::protocol_definitions::FwVersion,
            _offer: &embedded_cfu_protocol::protocol_definitions::FwUpdateOffer,
        ) -> embedded_cfu_protocol::protocol_definitions::FwUpdateOfferResponse {
            // For this example, we always accept the new version
            FwUpdateOfferResponse::new_accept(HostToken::Driver)
        }
    }

    pub type Wrapper<'a> = type_c_service::wrapper::ControllerWrapper<'a, 1, Controller<'a>, Validator>;
}

mod debug {
    use embedded_services::{
        comms::{self, Endpoint, EndpointID, Internal},
        info,
        type_c::comms::DebugAccessoryMessage,
    };

    pub struct Listener {
        pub tp: Endpoint,
    }

    impl Listener {
        pub fn new() -> Self {
            Self {
                tp: Endpoint::uninit(EndpointID::Internal(Internal::Usbc)),
            }
        }
    }

    impl comms::MailboxDelegate for Listener {
        fn receive(&self, message: &comms::Message) -> Result<(), comms::MailboxDelegateError> {
            if let Some(message) = message.data.get::<DebugAccessoryMessage>() {
                if message.connected {
                    info!("Port{}: Debug accessory connected", message.port.0);
                } else {
                    info!("Port{}: Debug accessory disconnected", message.port.0);
                }
            }

            Ok(())
        }
    }
}

#[embassy_executor::task]
async fn controller_task(state: &'static test_controller::ControllerState) {
    static WRAPPER: OnceLock<test_controller::Wrapper> = OnceLock::new();

    let controller = test_controller::Controller::new(state);
    let wrapper = WRAPPER.get_or_init(|| {
        test_controller::Wrapper::new(
            embedded_services::type_c::controller::Device::new(CONTROLLER0, &[PORT0, PORT0]),
            [policy::device::Device::new(POWER0)],
            embedded_services::cfu::component::CfuDevice::new(0x00),
            controller,
            crate::test_controller::Validator,
        )
    });

    wrapper.register().await.unwrap();

    loop {
        wrapper.process().await;
    }
}

#[embassy_executor::task]
async fn task(spawner: Spawner) {
    embedded_services::init().await;

    controller::init();

    // Register debug accessory listener
    static LISTENER: OnceLock<debug::Listener> = OnceLock::new();
    let listener = LISTENER.get_or_init(debug::Listener::new);
    comms::register_endpoint(listener, &listener.tp).await.unwrap();

    static STATE: OnceLock<test_controller::ControllerState> = OnceLock::new();
    let state = STATE.get_or_init(test_controller::ControllerState::new);

    info!("Starting controller task");
    spawner.must_spawn(controller_task(state));
    // Wait for controller to be registered
    Timer::after_secs(1).await;

    info!("Simulating connection");
    state.connect_sink(Current::UsbDefault);
    Timer::after_millis(250).await;

    info!("Simulating disconnection");
    state.disconnect();
    Timer::after_millis(250).await;

    info!("Simulating debug accessory connection");
    state.connect_debug_accessory_source(Current::UsbDefault);
    Timer::after_millis(250).await;

    info!("Simulating debug accessory disconnection");
    state.disconnect();
    Timer::after_millis(250).await;
}

fn main() {
    env_logger::builder().filter_level(log::LevelFilter::Trace).init();

    static EXECUTOR: StaticCell<Executor> = StaticCell::new();
    let executor = EXECUTOR.init(Executor::new());
    executor.run(|spawner| {
        spawner.must_spawn(power_policy_service::task(Default::default()));
        spawner.must_spawn(type_c_service::task());
        spawner.must_spawn(task(spawner));
    });
}
