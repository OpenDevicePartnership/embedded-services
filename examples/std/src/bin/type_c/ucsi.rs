use embassy_executor::{Executor, Spawner};
use embassy_time::Timer;
use embedded_services::GlobalRawMutex;
use embedded_services::power::policy;
use embedded_services::type_c::ControllerId;
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
use type_c_service::service::config::Config;
use type_c_service::wrapper::backing::{ReferencedStorage, Storage};

const CONTROLLER0: ControllerId = ControllerId(0);
const PORT0: GlobalPortId = GlobalPortId(0);
const POWER0: policy::DeviceId = policy::DeviceId(0);
const CFU0: u8 = 0x00;

#[embassy_executor::task]
async fn controller_task() {
    static STORAGE: StaticCell<Storage<1, GlobalRawMutex>> = StaticCell::new();
    let storage = STORAGE.init(Storage::new(CONTROLLER0, CFU0, [(PORT0, POWER0)]));
    static REFERENCED: StaticCell<ReferencedStorage<1, GlobalRawMutex>> = StaticCell::new();
    let referenced = REFERENCED.init(storage.create_referenced());

    static STATE0: StaticCell<mock_controller::ControllerState> = StaticCell::new();
    let state0 = STATE0.init(mock_controller::ControllerState::new());
    let controller0 = mock_controller::Controller::new(state0);
    static WRAPPER0: StaticCell<mock_controller::Wrapper> = StaticCell::new();
    let wrapper0 = WRAPPER0.init(
        mock_controller::Wrapper::try_new(controller0, referenced, mock_controller::Validator)
            .expect("Failed to create wrapper"),
    );

    wrapper0.register().await.unwrap();

    loop {
        if let Err(e) = wrapper0.process_next_event().await {
            error!("Error processing wrapper: {e:#?}");
        }
    }
}

#[embassy_executor::task]
async fn opm_task() {
    info!("Resetting PPM...");
    let response: UcsiResponseResult = execute_ucsi_command(Command::PpmCommand(ppm::Command::PpmReset))
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
    let response: UcsiResponseResult = execute_ucsi_command(Command::PpmCommand(ppm::Command::SetNotificationEnable(
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
    let response: UcsiResponseResult =
        execute_ucsi_command(Command::PpmCommand(ppm::Command::AckCcCi(ppm::ack_cc_ci::Args {
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

    info!("Get PPM capabilities...");
    let response: UcsiResponseResult = execute_ucsi_command(Command::PpmCommand(ppm::Command::GetCapability))
        .await
        .into();
    let response = response.unwrap();
    if !response.cci.cmd_complete() || response.cci.error() {
        error!("Get PPM capabilities failed: {response:?}");
    } else {
        info!("Get PPM capabilities successful: {:?}", response.data);
    }

    info!("Sending command complete ack...");
    let response: UcsiResponseResult =
        execute_ucsi_command(Command::PpmCommand(ppm::Command::AckCcCi(ppm::ack_cc_ci::Args {
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

    info!("Get connector capability...");
    let response: UcsiResponseResult = execute_ucsi_command(Command::LpmCommand(lpm::GlobalCommand::new(
        GlobalPortId(0),
        lpm::CommandData::GetConnectorCapability,
    )))
    .await
    .into();
    let response = response.unwrap();
    if !response.cci.cmd_complete() || response.cci.error() {
        error!("Get connector capability failed: {:?}", response.cci);
    } else {
        info!("Get connector capability successful: {:?}", response.data);
    }

    info!("Sending command complete ack...");
    let response: UcsiResponseResult =
        execute_ucsi_command(Command::PpmCommand(ppm::Command::AckCcCi(ppm::ack_cc_ci::Args {
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

    info!("Get connector status...");
    let response: UcsiResponseResult = execute_ucsi_command(Command::LpmCommand(lpm::GlobalCommand::new(
        GlobalPortId(0),
        lpm::CommandData::GetConnectorStatus,
    )))
    .await
    .into();
    let response = response.unwrap();
    if !response.cci.cmd_complete() || response.cci.error() {
        error!("Get connector status failed: {:?}", response.cci);
    } else {
        info!("Get connector status successful: {:?}", response.data);
    }
}

#[embassy_executor::task]
async fn task(spawner: Spawner) {
    info!("Starting main task");

    embedded_services::init().await;

    spawner.must_spawn(type_c_service::task(Config {
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
    }));
    spawner.must_spawn(controller_task());

    // Wait for the controller to initialize and register itself
    Timer::after_millis(500).await;
    spawner.must_spawn(opm_task());
}

fn main() {
    env_logger::builder().filter_level(log::LevelFilter::Trace).init();

    static EXECUTOR: StaticCell<Executor> = StaticCell::new();
    let executor = EXECUTOR.init(Executor::new());
    executor.run(|spawner| {
        spawner.must_spawn(task(spawner));
    });
}
