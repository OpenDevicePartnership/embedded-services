#![allow(unused_imports)]
use cfu_service::CfuClient;
use embassy_executor::{Executor, Spawner};
use embassy_sync::channel::{Channel, DynamicReceiver, DynamicSender};
use embassy_sync::mutex::Mutex;
use embassy_sync::once_lock::OnceLock;
use embassy_sync::pubsub::{DynImmediatePublisher, DynSubscriber, PubSubChannel};
use embedded_services::IntrusiveList;
use embedded_services::event::{MapSender, NoopSender};
use embedded_services::{GlobalRawMutex, event};
use embedded_usb_pd::ucsi::lpm::get_connector_capability::OperationModeFlags;
use embedded_usb_pd::ucsi::ppm::ack_cc_ci::Ack;
use embedded_usb_pd::ucsi::ppm::get_capability::ResponseData as UcsiCapabilities;
use embedded_usb_pd::ucsi::ppm::set_notification_enable::NotificationEnable;
use embedded_usb_pd::ucsi::{Command, lpm, ppm};
use embedded_usb_pd::{GlobalPortId, LocalPortId};
use log::*;
use power_policy_interface::capability::PowerCapability;
use power_policy_interface::charger::mock::ChargerType;
use power_policy_interface::psu;
use power_policy_service::psu::PsuEventReceivers;
use power_policy_service::service::registration::ArrayRegistration;
use static_cell::StaticCell;
use std_examples::type_c::mock_controller::{self, InterruptReceiver, Port};
use type_c_interface::controller::ControllerId;
use type_c_interface::port::event::PortEventBitfield;
use type_c_interface::service::event::{PortEvent as ServicePortEvent, PortEventData as ServicePortEventData};
use type_c_service::controller::event::Event as PortEvent;
use type_c_service::controller::event_receiver::InterruptReceiver as _;
use type_c_service::controller::event_receiver::{EventReceiver as PortEventReceiver, PortEventSplitter};
use type_c_service::controller::macros::PortComponents;
use type_c_service::controller::state::SharedState;
use type_c_service::define_controller_port_static_cell_channel;
use type_c_service::service::Service;
use type_c_service::service::config::Config;
use type_c_service::service::registration::PortData;

type ControllerType = Mutex<GlobalRawMutex, mock_controller::Controller<'static>>;
type PortType = Mutex<GlobalRawMutex, Port<'static>>;

type PowerPolicySenderType = MapSender<
    power_policy_interface::service::event::Event<'static, PortType>,
    power_policy_interface::service::event::EventData,
    DynImmediatePublisher<'static, power_policy_interface::service::event::EventData>,
    fn(
        power_policy_interface::service::event::Event<'static, PortType>,
    ) -> power_policy_interface::service::event::EventData,
>;

type PowerPolicyReceiverType = DynSubscriber<'static, power_policy_interface::service::event::EventData>;

type PowerPolicyServiceType = Mutex<
    GlobalRawMutex,
    power_policy_service::service::Service<
        'static,
        ArrayRegistration<'static, PortType, 2, PowerPolicySenderType, 1, ChargerType, 0>,
    >,
>;

const PORT_COUNT: usize = 2;
type TypeCServiceSenderType = NoopSender;
type PortReceiverType = DynamicReceiver<'static, type_c_interface::service::event::PortEventData>;
type TypeCServiceEventReceiverType = type_c_service::service::event_receiver::ArrayEventReceiver<
    'static,
    PORT_COUNT,
    PortType,
    PortReceiverType,
    PowerPolicyReceiverType,
>;
type TypeCRegistrationType =
    type_c_service::service::registration::ArrayRegistration<'static, PortType, PORT_COUNT, TypeCServiceSenderType, 1>;
type TypeCServiceType = Service<'static, TypeCRegistrationType>;
type SharedStateType = Mutex<GlobalRawMutex, SharedState>;
type PortEventReceiverType = PortEventReceiver<
    'static,
    SharedStateType,
    DynamicReceiver<'static, PortEventBitfield>,
    DynamicReceiver<'static, type_c_service::controller::event::Loopback>,
>;

#[embassy_executor::task]
async fn opm_task(_state: [&'static mock_controller::ControllerState; PORT_COUNT]) {
    // TODO: migrate this logic to an integration test when we move away from 'static lifetimes.
    /*const CAPABILITY: PowerCapability = PowerCapability {
        voltage_mv: 20000,
        current_ma: 5000,
    };

    info!("Resetting PPM...");
    let response: UcsiResponseResult = context
        .execute_ucsi_command_external(Command::PpmCommand(ppm::Command::PpmReset))
        .await
        .into();
    let response = response.unwrap();
    if !response.cci.reset_complete() || response.cci.error() {
        error!("PPM reset failed: {:?}", response.cci);
    } else {
        info!("PPM reset successful");
    }

    info!("Set Notification enable...");
    let mut notifications = NotificationEnable::default();
    notifications.set_cmd_complete(true);
    notifications.set_connect_change(true);
    let response: UcsiResponseResult = context
        .execute_ucsi_command_external(Command::PpmCommand(ppm::Command::SetNotificationEnable(
            ppm::set_notification_enable::Args {
                notification_enable: notifications,
            },
        )))
        .await
        .into();
    let response = response.unwrap();
    if !response.cci.cmd_complete() || response.cci.error() {
        error!("Set Notification enable failed: {:?}", response.cci);
    } else {
        info!("Set Notification enable successful");
    }

    info!("Sending command complete ack...");
    let response: UcsiResponseResult = context
        .execute_ucsi_command_external(Command::PpmCommand(ppm::Command::AckCcCi(ppm::ack_cc_ci::Args {
            ack: *Ack::default().set_command_complete(true),
        })))
        .await
        .into();
    let response = response.unwrap();
    if !response.cci.ack_command() || response.cci.error() {
        error!("Sending command complete ack failed: {:?}", response.cci);
    } else {
        info!("Sending command complete ack successful");
    }

    info!("Connecting sink on port 0");
    state[0].connect_sink(CAPABILITY, false).await;
    info!("Connecting sink on port 1");
    state[1].connect_sink(CAPABILITY, false).await;

    // Ensure connect flow has time to complete
    embassy_time::Timer::after_millis(1000).await;

    info!("Port 0: Get connector status...");
    let response: UcsiResponseResult = context
        .execute_ucsi_command_external(Command::LpmCommand(lpm::GlobalCommand::new(
            GlobalPortId(0),
            lpm::CommandData::GetConnectorStatus,
        )))
        .await
        .into();
    let response = response.unwrap();
    if !response.cci.cmd_complete() || response.cci.error() {
        error!("Get connector status failed: {:?}", response.cci);
    } else {
        info!(
            "Get connector status successful, connector change: {:?}",
            response.cci.connector_change()
        );
    }

    info!("Sending command complete ack...");
    let response: UcsiResponseResult = context
        .execute_ucsi_command_external(Command::PpmCommand(ppm::Command::AckCcCi(ppm::ack_cc_ci::Args {
            ack: *Ack::default().set_command_complete(true).set_connector_change(true),
        })))
        .await
        .into();
    let response = response.unwrap();
    if !response.cci.ack_command() || response.cci.error() {
        error!("Sending command complete ack failed: {:?}", response.cci);
    } else {
        info!(
            "Sending command complete ack successful, connector change:  {:?}",
            response.cci.connector_change()
        );
    }

    info!("Port 1: Get connector status...");
    let response: UcsiResponseResult = context
        .execute_ucsi_command_external(Command::LpmCommand(lpm::GlobalCommand::new(
            GlobalPortId(1),
            lpm::CommandData::GetConnectorStatus,
        )))
        .await
        .into();
    let response = response.unwrap();
    if !response.cci.cmd_complete() || response.cci.error() {
        error!("Get connector status failed: {:?}", response.cci);
    } else {
        info!(
            "Get connector status successful, connector change: {:?}",
            response.cci.connector_change()
        );
    }

    info!("Sending command complete ack...");
    let response: UcsiResponseResult = context
        .execute_ucsi_command_external(Command::PpmCommand(ppm::Command::AckCcCi(ppm::ack_cc_ci::Args {
            ack: *Ack::default().set_command_complete(true).set_connector_change(true),
        })))
        .await
        .into();
    let response = response.unwrap();
    if !response.cci.ack_command() || response.cci.error() {
        error!("Sending command complete ack failed: {:?}", response.cci);
    } else {
        info!(
            "Sending command complete ack successful, connector change:  {:?}",
            response.cci.connector_change()
        );
    }*/
}

#[embassy_executor::task(pool_size = 2)]
async fn port_task(mut event_receiver: PortEventReceiverType, port: &'static PortType) {
    loop {
        let event = event_receiver.wait_event().await;
        let output = port.lock().await.process_event(event).await;
        if let Err(e) = output {
            error!("Error processing event: {e:?}");
        }
    }
}

#[embassy_executor::task(pool_size = 2)]
async fn interrupt_splitter_task(
    mut interrupt_receiver: InterruptReceiver<'static>,
    mut interrupt_splitter: PortEventSplitter<1, DynamicSender<'static, PortEventBitfield>>,
) -> ! {
    loop {
        let interrupts = interrupt_receiver.wait_interrupt().await;
        interrupt_splitter.process_interrupts(interrupts).await;
    }
}

#[embassy_executor::task]
async fn power_policy_task(
    psu_events: PsuEventReceivers<'static, 2, PortType, DynamicReceiver<'static, psu::event::EventData>>,
    power_policy: &'static PowerPolicyServiceType,
) {
    power_policy_service::service::task::psu_task(psu_events, power_policy).await;
}

#[embassy_executor::task]
async fn type_c_service_task(
    service: &'static Mutex<GlobalRawMutex, TypeCServiceType>,
    event_receiver: TypeCServiceEventReceiverType,
) {
    type_c_service::task::task(service, event_receiver).await;
}

#[embassy_executor::task]
async fn task(spawner: Spawner) {
    info!("Starting main task");

    embedded_services::init().await;

    static STATE0: StaticCell<mock_controller::ControllerState> = StaticCell::new();
    let state0 = STATE0.init(mock_controller::ControllerState::new());
    static CONTROLLER0: StaticCell<ControllerType> = StaticCell::new();
    let controller0 = CONTROLLER0.init(Mutex::new(mock_controller::Controller::new(state0, "Controller0")));

    define_controller_port_static_cell_channel!(pub(self), port0, GlobalRawMutex, Mutex<GlobalRawMutex, mock_controller::Controller<'static>>);
    let PortComponents {
        port: port0,
        power_policy_receiver: policy_receiver0,
        event_receiver: event_receiver0,
        interrupt_sender: port0_interrupt_sender,
        type_c_receiver: type_c_receiver0,
    } = port0::create("PD0", LocalPortId(0), Default::default(), controller0);

    static STATE1: StaticCell<mock_controller::ControllerState> = StaticCell::new();
    let state1 = STATE1.init(mock_controller::ControllerState::new());
    static CONTROLLER1: StaticCell<ControllerType> = StaticCell::new();
    let controller1 = CONTROLLER1.init(Mutex::new(mock_controller::Controller::new(state1, "Controller1")));

    define_controller_port_static_cell_channel!(pub(self), port1, GlobalRawMutex, Mutex<GlobalRawMutex, mock_controller::Controller<'static>>);
    let PortComponents {
        port: port1,
        power_policy_receiver: policy_receiver1,
        event_receiver: event_receiver1,
        interrupt_sender: port1_interrupt_sender,
        type_c_receiver: type_c_receiver1,
    } = port1::create("PD1", LocalPortId(0), Default::default(), controller1);

    // Create power policy service
    // The service is the only receiver and we only use a DynImmediatePublisher, which doesn't take a publisher slot
    static POWER_POLICY_CHANNEL: StaticCell<
        PubSubChannel<GlobalRawMutex, power_policy_interface::service::event::EventData, 4, 1, 0>,
    > = StaticCell::new();

    let power_policy_channel = POWER_POLICY_CHANNEL.init(PubSubChannel::new());
    let power_policy_sender: PowerPolicySenderType =
        MapSender::new(power_policy_channel.dyn_immediate_publisher(), |e| e.into());
    // Guaranteed to not panic since we initialized the channel above
    let power_policy_subscriber = power_policy_channel.dyn_subscriber().unwrap();

    let power_policy_registration = ArrayRegistration {
        psus: [port0, port1],
        service_senders: [power_policy_sender],
        chargers: [],
    };

    static POWER_SERVICE: StaticCell<PowerPolicyServiceType> = StaticCell::new();
    let power_service = POWER_SERVICE.init(Mutex::new(power_policy_service::service::Service::new(
        power_policy_registration,
        power_policy_service::service::config::Config::default(),
    )));

    // Create type-c service
    static TYPE_C_SERVICE: StaticCell<Mutex<GlobalRawMutex, TypeCServiceType>> = StaticCell::new();
    let type_c_service = TYPE_C_SERVICE.init(Mutex::new(Service::create(
        Config {
            ucsi_capabilities: UcsiCapabilities {
                num_connectors: 2,
                bcd_usb_pd_spec: 0x0300,
                bcd_type_c_spec: 0x0200,
                bcd_battery_charging_spec: 0x0120,
                ..Default::default()
            },
            ucsi_port_capabilities: Some(
                *lpm::get_connector_capability::ResponseData::default()
                    .set_operation_mode(
                        *OperationModeFlags::default()
                            .set_drp(true)
                            .set_usb2(true)
                            .set_usb3(true),
                    )
                    .set_consumer(true)
                    .set_provider(true)
                    .set_swap_to_dfp(true)
                    .set_swap_to_snk(true)
                    .set_swap_to_src(true),
            ),
            ..Default::default()
        },
        TypeCRegistrationType {
            ports: [port0, port1],
            service_senders: [NoopSender],
            port_data: [
                PortData {
                    local_port: Some(LocalPortId(0)),
                },
                PortData {
                    local_port: Some(LocalPortId(1)),
                },
            ],
        },
    )));

    spawner.spawn(
        power_policy_task(
            PsuEventReceivers::new([port0, port1], [policy_receiver0, policy_receiver1]),
            power_service,
        )
        .expect("Failed to create power policy task"),
    );

    spawner.spawn(
        type_c_service_task(
            type_c_service,
            TypeCServiceEventReceiverType::new(
                [port0, port1],
                [type_c_receiver0, type_c_receiver1],
                power_policy_subscriber,
            ),
        )
        .expect("Failed to create type-c service task"),
    );
    spawner.spawn(port_task(event_receiver0, port0).expect("Failed to create wrapper0 task"));
    spawner.spawn(
        interrupt_splitter_task(
            state0.create_interrupt_receiver(),
            PortEventSplitter::new([port0_interrupt_sender]),
        )
        .expect("Failed to create interrupt splitter 0 task"),
    );
    spawner.spawn(port_task(event_receiver1, port1).expect("Failed to create wrapper1 task"));
    spawner.spawn(
        interrupt_splitter_task(
            state1.create_interrupt_receiver(),
            PortEventSplitter::new([port1_interrupt_sender]),
        )
        .expect("Failed to create interrupt splitter 1 task"),
    );
    spawner.spawn(opm_task([state0, state1]).expect("Failed to create opm task"));
}

fn main() {
    env_logger::builder().filter_level(log::LevelFilter::Trace).init();

    static EXECUTOR: StaticCell<Executor> = StaticCell::new();
    let executor = EXECUTOR.init(Executor::new());

    executor.run(|spawner| {
        spawner.spawn(task(spawner).expect("Failed to create task"));
    });
}
