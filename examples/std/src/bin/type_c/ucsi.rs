#![allow(unused_imports)]
use crate::mock_controller::Wrapper;
use cfu_service::CfuClient;
use embassy_executor::{Executor, Spawner};
use embassy_sync::channel::{Channel, DynamicReceiver, DynamicSender};
use embassy_sync::mutex::Mutex;
use embassy_sync::once_lock::OnceLock;
use embassy_sync::pubsub::{DynImmediatePublisher, DynSubscriber, PubSubChannel};
use embedded_services::GlobalRawMutex;
use embedded_services::IntrusiveList;
use embedded_services::event::MapSender;
use embedded_usb_pd::GlobalPortId;
use embedded_usb_pd::ucsi::lpm::get_connector_capability::OperationModeFlags;
use embedded_usb_pd::ucsi::ppm::ack_cc_ci::Ack;
use embedded_usb_pd::ucsi::ppm::get_capability::ResponseData as UcsiCapabilities;
use embedded_usb_pd::ucsi::ppm::set_notification_enable::NotificationEnable;
use embedded_usb_pd::ucsi::{Command, lpm, ppm};
use log::*;
use power_policy_interface::capability::PowerCapability;
use power_policy_interface::psu;
use power_policy_service::psu::ArrayEventReceivers;
use power_policy_service::service::registration::ArrayRegistration;
use static_cell::StaticCell;
use std_examples::type_c::mock_controller;
use type_c_interface::port::{ControllerId, PortRegistration};
use type_c_interface::service::context::Context;
use type_c_interface::service::event::PortEvent as ServicePortEvent;
use type_c_service::service::config::Config;
use type_c_service::service::{EventReceiver, Service};
use type_c_service::wrapper::backing::Storage;
use type_c_service::wrapper::event_receiver::ArrayPortEventReceivers;
use type_c_service::wrapper::proxy::PowerProxyDevice;

const CHANNEL_CAPACITY: usize = 4;
const NUM_PD_CONTROLLERS: usize = 2;
const CONTROLLER0_ID: ControllerId = ControllerId(0);
const CONTROLLER1_ID: ControllerId = ControllerId(1);
const PORT0_ID: GlobalPortId = GlobalPortId(0);
const PORT1_ID: GlobalPortId = GlobalPortId(1);
const CFU0_ID: u8 = 0x00;
const CFU1_ID: u8 = 0x01;

type DeviceType = Mutex<GlobalRawMutex, PowerProxyDevice<'static>>;

type PowerPolicySenderType = MapSender<
    power_policy_interface::service::event::Event<'static, DeviceType>,
    power_policy_interface::service::event::EventData,
    DynImmediatePublisher<'static, power_policy_interface::service::event::EventData>,
    fn(
        power_policy_interface::service::event::Event<'static, DeviceType>,
    ) -> power_policy_interface::service::event::EventData,
>;

type PowerPolicyReceiverType = DynSubscriber<'static, power_policy_interface::service::event::EventData>;

type PowerPolicyServiceType = Mutex<
    GlobalRawMutex,
    power_policy_service::service::Service<
        'static,
        ArrayRegistration<'static, DeviceType, 2, PowerPolicySenderType, 1>,
    >,
>;

type ServiceType = Service<'static>;

#[embassy_executor::task]
async fn opm_task(_context: &'static Context, _state: [&'static mock_controller::ControllerState; NUM_PD_CONTROLLERS]) {
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
async fn wrapper_task(
    mut event_receiver: ArrayPortEventReceivers<'static, 1, mock_controller::InterruptReceiver<'static>>,
    wrapper: &'static mock_controller::Wrapper<'static>,
) {
    loop {
        let event = event_receiver.wait_event().await;

        let output = wrapper
            .process_event(
                &mut event_receiver.sink_ready_timeout,
                &mut event_receiver.cfu_event_receiver,
                event,
            )
            .await;
        if let Err(e) = output {
            error!("Error processing event: {e:?}");
        }
        let output = output.unwrap();
        if let Err(e) = wrapper.finalize(&mut event_receiver.power_proxies, output).await {
            error!("Error finalizing output: {e:#?}");
        }
    }
}

#[embassy_executor::task]
async fn power_policy_task(
    psu_events: ArrayEventReceivers<'static, 2, DeviceType, DynamicReceiver<'static, psu::event::EventData>>,
    power_policy: &'static PowerPolicyServiceType,
) {
    power_policy_service::service::task::task(psu_events, power_policy).await;
}

#[embassy_executor::task]
async fn type_c_service_task(
    service: &'static Mutex<GlobalRawMutex, ServiceType>,
    event_receiver: EventReceiver<'static, PowerPolicyReceiverType>,
    wrappers: [&'static Wrapper<'static>; NUM_PD_CONTROLLERS],
    cfu_client: &'static CfuClient,
) {
    info!("Starting type-c task");
    type_c_service::task::task(service, event_receiver, wrappers, cfu_client).await;
}

#[embassy_executor::task]
async fn task(spawner: Spawner) {
    info!("Starting main task");

    embedded_services::init().await;

    static CONTROLLER_CONTEXT: StaticCell<Context> = StaticCell::new();
    let controller_context = CONTROLLER_CONTEXT.init(Context::new());

    static PORT0_CHANNEL: Channel<GlobalRawMutex, ServicePortEvent, CHANNEL_CAPACITY> = Channel::new();
    static STORAGE0: StaticCell<Storage<1, GlobalRawMutex>> = StaticCell::new();
    let storage0 = STORAGE0.init(Storage::new(
        controller_context,
        CONTROLLER0_ID,
        CFU0_ID,
        [PortRegistration {
            id: PORT0_ID,
            sender: PORT0_CHANNEL.dyn_sender(),
            receiver: PORT0_CHANNEL.dyn_receiver(),
        }],
    ));

    static POLICY_CHANNEL0: StaticCell<Channel<GlobalRawMutex, psu::event::EventData, 2>> = StaticCell::new();
    let policy_channel0 = POLICY_CHANNEL0.init(Channel::new());
    let policy_sender0 = policy_channel0.dyn_sender();
    let policy_receiver0 = policy_channel0.dyn_receiver();

    static INTERMEDIATE0: StaticCell<
        type_c_service::wrapper::backing::IntermediateStorage<
            1,
            GlobalRawMutex,
            DynamicSender<'_, psu::event::EventData>,
        >,
    > = StaticCell::new();
    let (intermediate0, power_event_receivers0) = storage0
        .try_create_intermediate([("Pd0", policy_sender0)])
        .expect("Failed to create intermediate storage");
    let intermediate0 = INTERMEDIATE0.init(intermediate0);

    static REFERENCED0: StaticCell<
        type_c_service::wrapper::backing::ReferencedStorage<
            1,
            GlobalRawMutex,
            DynamicSender<'_, psu::event::EventData>,
        >,
    > = StaticCell::new();
    let referenced0 = REFERENCED0.init(
        intermediate0
            .try_create_referenced()
            .expect("Failed to create referenced storage"),
    );

    static STATE0: StaticCell<mock_controller::ControllerState> = StaticCell::new();
    let state0 = STATE0.init(mock_controller::ControllerState::new());
    let event_receiver0 = ArrayPortEventReceivers::new(
        state0.create_interrupt_receiver(),
        power_event_receivers0,
        &referenced0.pd_controller,
        &storage0.cfu_device,
    );
    static CONTROLLER0: StaticCell<Mutex<GlobalRawMutex, mock_controller::Controller>> = StaticCell::new();
    let controller0 = CONTROLLER0.init(Mutex::new(mock_controller::Controller::new(state0)));
    static WRAPPER0: StaticCell<mock_controller::Wrapper> = StaticCell::new();
    let wrapper0 = WRAPPER0.init(mock_controller::Wrapper::new(
        controller0,
        Default::default(),
        referenced0,
        mock_controller::Validator,
    ));

    static POLICY_CHANNEL1: StaticCell<Channel<GlobalRawMutex, psu::event::EventData, 2>> = StaticCell::new();
    let policy_channel1 = POLICY_CHANNEL1.init(Channel::new());
    let policy_sender1 = policy_channel1.dyn_sender();
    let policy_receiver1 = policy_channel1.dyn_receiver();

    static PORT1_CHANNEL: Channel<GlobalRawMutex, ServicePortEvent, CHANNEL_CAPACITY> = Channel::new();
    static STORAGE1: StaticCell<Storage<1, GlobalRawMutex>> = StaticCell::new();
    let storage1 = STORAGE1.init(Storage::new(
        controller_context,
        CONTROLLER1_ID,
        CFU1_ID,
        [PortRegistration {
            id: PORT1_ID,
            sender: PORT1_CHANNEL.dyn_sender(),
            receiver: PORT1_CHANNEL.dyn_receiver(),
        }],
    ));
    static INTERMEDIATE1: StaticCell<
        type_c_service::wrapper::backing::IntermediateStorage<
            1,
            GlobalRawMutex,
            DynamicSender<'_, psu::event::EventData>,
        >,
    > = StaticCell::new();
    let (intermediate1, power_event_receivers1) = storage1
        .try_create_intermediate([("Pd1", policy_sender1)])
        .expect("Failed to create intermediate storage");
    let intermediate1 = INTERMEDIATE1.init(intermediate1);

    static REFERENCED1: StaticCell<
        type_c_service::wrapper::backing::ReferencedStorage<
            1,
            GlobalRawMutex,
            DynamicSender<'_, psu::event::EventData>,
        >,
    > = StaticCell::new();
    let referenced1 = REFERENCED1.init(
        intermediate1
            .try_create_referenced()
            .expect("Failed to create referenced storage"),
    );

    static STATE1: StaticCell<mock_controller::ControllerState> = StaticCell::new();
    let state1 = STATE1.init(mock_controller::ControllerState::new());
    let event_receiver1 = ArrayPortEventReceivers::new(
        state1.create_interrupt_receiver(),
        power_event_receivers1,
        &referenced1.pd_controller,
        &storage1.cfu_device,
    );
    static CONTROLLER1: StaticCell<Mutex<GlobalRawMutex, mock_controller::Controller>> = StaticCell::new();
    let controller1 = CONTROLLER1.init(Mutex::new(mock_controller::Controller::new(state1)));
    static WRAPPER1: StaticCell<mock_controller::Wrapper> = StaticCell::new();
    let wrapper1 = WRAPPER1.init(mock_controller::Wrapper::new(
        controller1,
        Default::default(),
        referenced1,
        mock_controller::Validator,
    ));

    // Create power policy service
    static POWER_SERVICE_CONTEXT: StaticCell<power_policy_service::service::context::Context> = StaticCell::new();
    let power_service_context = POWER_SERVICE_CONTEXT.init(power_policy_service::service::context::Context::new());

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
        psus: [&wrapper0.ports[0].proxy, &wrapper1.ports[0].proxy],
        service_senders: [power_policy_sender],
    };

    static POWER_SERVICE: StaticCell<PowerPolicyServiceType> = StaticCell::new();
    let power_service = POWER_SERVICE.init(Mutex::new(power_policy_service::service::Service::new(
        power_policy_registration,
        power_service_context,
        power_policy_service::service::config::Config::default(),
    )));

    // Create type-c service
    static TYPE_C_SERVICE: StaticCell<Mutex<GlobalRawMutex, ServiceType>> = StaticCell::new();
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
        controller_context,
    )));

    // Spin up CFU service
    static CFU_CLIENT: OnceLock<CfuClient> = OnceLock::new();
    let cfu_client = CfuClient::new(&CFU_CLIENT).await;

    spawner.spawn(
        power_policy_task(
            ArrayEventReceivers::new(
                [&wrapper0.ports[0].proxy, &wrapper1.ports[0].proxy],
                [policy_receiver0, policy_receiver1],
            ),
            power_service,
        )
        .expect("Failed to create power policy task"),
    );

    spawner.spawn(
        type_c_service_task(
            type_c_service,
            EventReceiver::new(controller_context, power_policy_subscriber),
            [wrapper0, wrapper1],
            cfu_client,
        )
        .expect("Failed to create type-c service task"),
    );
    spawner.spawn(wrapper_task(event_receiver0, wrapper0).expect("Failed to create wrapper0 task"));
    spawner.spawn(wrapper_task(event_receiver1, wrapper1).expect("Failed to create wrapper1 task"));
    spawner.spawn(opm_task(controller_context, [state0, state1]).expect("Failed to create opm task"));
}

fn main() {
    env_logger::builder().filter_level(log::LevelFilter::Trace).init();

    static EXECUTOR: StaticCell<Executor> = StaticCell::new();
    let executor = EXECUTOR.init(Executor::new());

    executor.run(|spawner| {
        spawner.spawn(task(spawner).expect("Failed to create task"));
    });
}
