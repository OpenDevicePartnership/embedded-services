use crate::mock_controller::Wrapper;
use cfu_service::CfuClient;
use embassy_executor::{Executor, Spawner};
use embassy_sync::channel::{Channel, DynamicReceiver, DynamicSender};
use embassy_sync::mutex::Mutex;
use embassy_sync::once_lock::OnceLock;
use embassy_sync::pubsub::PubSubChannel;
use embedded_services::GlobalRawMutex;
use embedded_services::IntrusiveList;
use embedded_usb_pd::GlobalPortId;
use embedded_usb_pd::ucsi::lpm::get_connector_capability::OperationModeFlags;
use embedded_usb_pd::ucsi::ppm::ack_cc_ci::Ack;
use embedded_usb_pd::ucsi::ppm::get_capability::ResponseData as UcsiCapabilities;
use embedded_usb_pd::ucsi::ppm::set_notification_enable::NotificationEnable;
use embedded_usb_pd::ucsi::{Command, lpm, ppm};
use log::*;
use power_policy_service::capability::PowerCapability;
use power_policy_service::psu;
use static_cell::StaticCell;
use std_examples::type_c::mock_controller;
use type_c_service::service::Service;
use type_c_service::service::config::Config;
use type_c_service::type_c::ControllerId;
use type_c_service::type_c::controller::Context;
use type_c_service::type_c::external::UcsiResponseResult;
use type_c_service::wrapper::backing::Storage;
use type_c_service::wrapper::proxy::PowerProxyDevice;

const NUM_PD_CONTROLLERS: usize = 2;
const CONTROLLER0_ID: ControllerId = ControllerId(0);
const CONTROLLER1_ID: ControllerId = ControllerId(1);
const PORT0_ID: GlobalPortId = GlobalPortId(0);
const PORT1_ID: GlobalPortId = GlobalPortId(1);
const CFU0_ID: u8 = 0x00;
const CFU1_ID: u8 = 0x01;

type DeviceType = Mutex<GlobalRawMutex, PowerProxyDevice<'static>>;

#[embassy_executor::task]
async fn opm_task(context: &'static Context, state: [&'static mock_controller::ControllerState; NUM_PD_CONTROLLERS]) {
    const CAPABILITY: PowerCapability = PowerCapability {
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
    }
}

#[embassy_executor::task(pool_size = 2)]
async fn wrapper_task(wrapper: &'static mock_controller::Wrapper<'static>) {
    loop {
        if let Err(e) = wrapper.process_next_event().await {
            error!("Error processing wrapper: {e:#?}");
        }
    }
}

#[embassy_executor::task]
async fn power_policy_task(
    psu_events: psu::event::EventReceivers<'static, 2, DeviceType, DynamicReceiver<'static, psu::event::EventData>>,
    power_policy: &'static Mutex<GlobalRawMutex, power_policy_service::service::Service<'static, DeviceType>>,
) {
    power_policy_service::service::task::task(psu_events, power_policy).await;
}

#[embassy_executor::task]
async fn type_c_service_task(
    service: &'static Service<'static, DeviceType>,
    wrappers: [&'static Wrapper<'static>; NUM_PD_CONTROLLERS],
    power_policy_context: &'static power_policy_service::service::context::Context,
    cfu_client: &'static CfuClient,
) {
    info!("Starting type-c task");
    type_c_service::task::task(service, wrappers, power_policy_context, cfu_client).await;
}

#[embassy_executor::task]
async fn task(spawner: Spawner) {
    info!("Starting main task");

    embedded_services::init().await;

    static CONTROLLER_CONTEXT: StaticCell<type_c_service::type_c::controller::Context> = StaticCell::new();
    let controller_context = CONTROLLER_CONTEXT.init(type_c_service::type_c::controller::Context::new());

    static CONTROLLER_LIST: StaticCell<IntrusiveList> = StaticCell::new();
    let controller_list = CONTROLLER_LIST.init(IntrusiveList::new());

    static STORAGE0: StaticCell<Storage<1, GlobalRawMutex>> = StaticCell::new();
    let storage0 = STORAGE0.init(Storage::new(controller_context, CONTROLLER0_ID, CFU0_ID, [PORT0_ID]));

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
    let intermediate0 = INTERMEDIATE0.init(
        storage0
            .try_create_intermediate([("Pd0", policy_sender0)])
            .expect("Failed to create intermediate storage"),
    );

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

    static STORAGE1: StaticCell<Storage<1, GlobalRawMutex>> = StaticCell::new();
    let storage1 = STORAGE1.init(Storage::new(controller_context, CONTROLLER1_ID, CFU1_ID, [PORT1_ID]));
    static INTERMEDIATE1: StaticCell<
        type_c_service::wrapper::backing::IntermediateStorage<
            1,
            GlobalRawMutex,
            DynamicSender<'_, psu::event::EventData>,
        >,
    > = StaticCell::new();
    let intermediate1 = INTERMEDIATE1.init(
        storage1
            .try_create_intermediate([("Pd1", policy_sender1)])
            .expect("Failed to create intermediate storage"),
    );

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

    static POWER_POLICY_PSU_REGISTRATION: StaticCell<[&DeviceType; 2]> = StaticCell::new();
    let psu_registration = POWER_POLICY_PSU_REGISTRATION.init([&wrapper0.ports[0].proxy, &wrapper1.ports[0].proxy]);

    static POWER_SERVICE: StaticCell<Mutex<GlobalRawMutex, power_policy_service::service::Service<DeviceType>>> =
        StaticCell::new();
    let power_service = POWER_SERVICE.init(Mutex::new(power_policy_service::service::Service::new(
        psu_registration,
        power_service_context,
        power_policy_service::service::config::Config::default(),
    )));

    // Create type-c service
    // The service is the only receiver and we only use a DynImmediatePublisher, which doesn't take a publisher slot
    static POWER_POLICY_CHANNEL: StaticCell<
        PubSubChannel<GlobalRawMutex, power_policy_service::service::event::Event<'static, DeviceType>, 4, 1, 0>,
    > = StaticCell::new();

    let power_policy_channel = POWER_POLICY_CHANNEL.init(PubSubChannel::new());
    let power_policy_publisher = power_policy_channel.dyn_immediate_publisher();
    // Guaranteed to not panic since we initialized the channel above
    let power_policy_subscriber = power_policy_channel.dyn_subscriber().unwrap();

    static TYPE_C_SERVICE: StaticCell<Service<'static, DeviceType>> = StaticCell::new();
    let type_c_service = TYPE_C_SERVICE.init(Service::create(
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
        controller_list,
        power_policy_publisher,
        power_policy_subscriber,
    ));

    // Spin up CFU service
    static CFU_CLIENT: OnceLock<CfuClient> = OnceLock::new();
    let cfu_client = CfuClient::new(&CFU_CLIENT).await;

    spawner.must_spawn(power_policy_task(
        psu::event::EventReceivers::new(
            [&wrapper0.ports[0].proxy, &wrapper1.ports[0].proxy],
            [policy_receiver0, policy_receiver1],
        ),
        power_service,
    ));

    spawner.must_spawn(type_c_service_task(
        type_c_service,
        [wrapper0, wrapper1],
        power_service_context,
        cfu_client,
    ));
    spawner.must_spawn(wrapper_task(wrapper0));
    spawner.must_spawn(wrapper_task(wrapper1));
    spawner.must_spawn(opm_task(controller_context, [state0, state1]));
}

fn main() {
    env_logger::builder().filter_level(log::LevelFilter::Trace).init();

    static EXECUTOR: StaticCell<Executor> = StaticCell::new();
    let executor = EXECUTOR.init(Executor::new());

    executor.run(|spawner| {
        spawner.must_spawn(task(spawner));
    });
}
