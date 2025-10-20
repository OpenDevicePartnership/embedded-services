use embassy_executor::{Executor, Spawner};
use embassy_sync::mutex::Mutex;
use embassy_sync::pubsub::PubSubChannel;
use embedded_services::GlobalRawMutex;
use embedded_services::IntrusiveList;
use embedded_services::power::policy::{self, PowerCapability};
use embedded_services::type_c::ControllerId;
use embedded_services::type_c::controller::Context;
use embedded_services::type_c::external::{UcsiResponseResult, execute_ucsi_command};
use embedded_usb_pd::GlobalPortId;
use embedded_usb_pd::ucsi::lpm::get_connector_capability::OperationModeFlags;
use embedded_usb_pd::ucsi::ppm::ack_cc_ci::Ack;
use embedded_usb_pd::ucsi::ppm::get_capability::ResponseData as UcsiCapabilities;
use embedded_usb_pd::ucsi::ppm::set_notification_enable::NotificationEnable;
use embedded_usb_pd::ucsi::{Command, lpm, ppm};
use log::*;
use static_cell::StaticCell;
use std_examples::type_c::mock_controller;
use type_c_service::service::Service;
use type_c_service::service::config::Config;
use type_c_service::wrapper::backing::{ReferencedStorage, Storage};

const CONTROLLER0_ID: ControllerId = ControllerId(0);
const CONTROLLER1_ID: ControllerId = ControllerId(1);
const PORT0_ID: GlobalPortId = GlobalPortId(0);
const POWER0_ID: policy::DeviceId = policy::DeviceId(0);
const PORT1_ID: GlobalPortId = GlobalPortId(1);
const POWER1_ID: policy::DeviceId = policy::DeviceId(1);
const CFU0_ID: u8 = 0x00;
const CFU1_ID: u8 = 0x01;

#[embassy_executor::task]
async fn opm_task(spawner: Spawner, context: &'static Context, controller_list: &'static IntrusiveList) {
    static STORAGE0: StaticCell<Storage<1, GlobalRawMutex>> = StaticCell::new();
    let storage0 = STORAGE0.init(Storage::new(context, CONTROLLER0_ID, CFU0_ID, [(PORT0_ID, POWER0_ID)]));
    static REFERENCED0: StaticCell<ReferencedStorage<1, GlobalRawMutex>> = StaticCell::new();
    let referenced0 = REFERENCED0.init(storage0.create_referenced());

    static STATE0: StaticCell<mock_controller::ControllerState> = StaticCell::new();
    let state0 = STATE0.init(mock_controller::ControllerState::new());
    static CONTROLLER0: StaticCell<Mutex<GlobalRawMutex, mock_controller::Controller>> = StaticCell::new();
    let controller0 = CONTROLLER0.init(Mutex::new(mock_controller::Controller::new(state0)));
    static WRAPPER0: StaticCell<mock_controller::Wrapper> = StaticCell::new();
    let wrapper0 = WRAPPER0.init(
        mock_controller::Wrapper::try_new(controller0, Default::default(), referenced0, mock_controller::Validator)
            .expect("Failed to create wrapper"),
    );
    spawner.must_spawn(wrapper_task(wrapper0, controller_list));

    static STORAGE1: StaticCell<Storage<1, GlobalRawMutex>> = StaticCell::new();
    let storage1 = STORAGE1.init(Storage::new(context, CONTROLLER1_ID, CFU1_ID, [(PORT1_ID, POWER1_ID)]));
    static REFERENCED1: StaticCell<ReferencedStorage<1, GlobalRawMutex>> = StaticCell::new();
    let referenced1 = REFERENCED1.init(storage1.create_referenced());

    static STATE1: StaticCell<mock_controller::ControllerState> = StaticCell::new();
    let state1 = STATE1.init(mock_controller::ControllerState::new());
    static CONTROLLER1: StaticCell<Mutex<GlobalRawMutex, mock_controller::Controller>> = StaticCell::new();
    let controller1 = CONTROLLER1.init(Mutex::new(mock_controller::Controller::new(state1)));
    static WRAPPER1: StaticCell<mock_controller::Wrapper> = StaticCell::new();
    let wrapper1 = WRAPPER1.init(
        mock_controller::Wrapper::try_new(controller1, Default::default(), referenced1, mock_controller::Validator)
            .expect("Failed to create wrapper"),
    );
    spawner.must_spawn(wrapper_task(wrapper1, controller_list));

    const CAPABILITY: PowerCapability = PowerCapability {
        voltage_mv: 20000,
        current_ma: 5000,
    };

    info!("Resetting PPM...");
    let response: UcsiResponseResult = execute_ucsi_command(context, Command::PpmCommand(ppm::Command::PpmReset))
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
    let response: UcsiResponseResult = execute_ucsi_command(
        context,
        Command::PpmCommand(ppm::Command::SetNotificationEnable(
            ppm::set_notification_enable::Args {
                notification_enable: notifications,
            },
        )),
    )
    .await
    .into();
    let response = response.unwrap();
    if !response.cci.cmd_complete() || response.cci.error() {
        error!("Set Notification enable failed: {:?}", response.cci);
    } else {
        info!("Set Notification enable successful");
    }

    info!("Sending command complete ack...");
    let response: UcsiResponseResult = execute_ucsi_command(
        context,
        Command::PpmCommand(ppm::Command::AckCcCi(ppm::ack_cc_ci::Args {
            ack: *Ack::default().set_command_complete(true),
        })),
    )
    .await
    .into();
    let response = response.unwrap();
    if !response.cci.ack_command() || response.cci.error() {
        error!("Sending command complete ack failed: {:?}", response.cci);
    } else {
        info!("Sending command complete ack successful");
    }

    info!("Connecting sinks on both ports");
    state0.connect_sink(CAPABILITY, false).await;
    state1.connect_sink(CAPABILITY, false).await;

    // Ensure connect flow has time to complete
    embassy_time::Timer::after_millis(1000).await;

    info!("Port 0: Get connector status...");
    let response: UcsiResponseResult = execute_ucsi_command(
        context,
        Command::LpmCommand(lpm::GlobalCommand::new(
            GlobalPortId(0),
            lpm::CommandData::GetConnectorStatus,
        )),
    )
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
    let response: UcsiResponseResult = execute_ucsi_command(
        context,
        Command::PpmCommand(ppm::Command::AckCcCi(ppm::ack_cc_ci::Args {
            ack: *Ack::default().set_command_complete(true).set_connector_change(true),
        })),
    )
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
    let response: UcsiResponseResult = execute_ucsi_command(
        context,
        Command::LpmCommand(lpm::GlobalCommand::new(
            GlobalPortId(1),
            lpm::CommandData::GetConnectorStatus,
        )),
    )
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
    let response: UcsiResponseResult = execute_ucsi_command(
        context,
        Command::PpmCommand(ppm::Command::AckCcCi(ppm::ack_cc_ci::Args {
            ack: *Ack::default().set_command_complete(true).set_connector_change(true),
        })),
    )
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
async fn wrapper_task(wrapper: &'static mock_controller::Wrapper<'static>, controller_list: &'static IntrusiveList) {
    wrapper.register(controller_list).await.unwrap();

    loop {
        if let Err(e) = wrapper.process_next_event().await {
            error!("Error processing wrapper: {e:#?}");
        }
    }
}

#[embassy_executor::task]
async fn service_task(config: Config, controller_context: &'static Context, controllers: &'static IntrusiveList) {
    info!("Starting type-c task");

    // The service is the only receiver and we only use a DynImmediatePublisher, which doesn't take a publisher slot
    static POWER_POLICY_CHANNEL: StaticCell<
        PubSubChannel<GlobalRawMutex, embedded_services::power::policy::CommsMessage, 4, 1, 0>,
    > = StaticCell::new();

    let power_policy_channel = POWER_POLICY_CHANNEL.init(PubSubChannel::new());
    let power_policy_publisher = power_policy_channel.dyn_immediate_publisher();
    // Guaranteed to not panic since we initialized the channel above
    let power_policy_subscriber = power_policy_channel.dyn_subscriber().unwrap();

    let service = Service::create(
        config,
        controller_context,
        controllers,
        power_policy_publisher,
        power_policy_subscriber,
    );

    static SERVICE: StaticCell<Service> = StaticCell::new();
    let service = SERVICE.init(service);

    if service.register_comms().is_err() {
        error!("Failed to register type-c service endpoint, service already registered?");
        return;
    }

    loop {
        let event = match service.wait_next(controllers).await {
            Ok(event) => event,
            Err(e) => {
                error!("Error waiting for next event: {e:?}");
                continue;
            }
        };

        // Note: must call process_event before so port status is cached for everything else
        if let Err(e) = service.process_event(event, controllers).await {
            error!("Type-C service processing error: {e:#?}");
        }
    }
}

#[embassy_executor::task]
async fn task(spawner: Spawner) {
    info!("Starting main task");

    embedded_services::init().await;

    static CONTROLLER_LIST: StaticCell<IntrusiveList> = StaticCell::new();
    let controller_list = CONTROLLER_LIST.init(IntrusiveList::new());
    static CONTEXT: StaticCell<embedded_services::type_c::controller::Context> = StaticCell::new();
    let context = CONTEXT.init(embedded_services::type_c::controller::Context::new());

    spawner.must_spawn(power_policy_service::task(Default::default()));
    spawner.must_spawn(service_task(
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
        },
        context,
        controller_list,
    ));
    spawner.must_spawn(opm_task(spawner, context, controller_list));
}

fn main() {
    env_logger::builder().filter_level(log::LevelFilter::Trace).init();

    static EXECUTOR: StaticCell<Executor> = StaticCell::new();
    let executor = EXECUTOR.init(Executor::new());
    executor.run(|spawner| {
        spawner.must_spawn(task(spawner));
    });
}
