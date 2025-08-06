use embassy_executor::{Executor, Spawner};
use embedded_services::type_c::controller;
use embedded_services::type_c::external::execute_ucsi_command;
use embedded_usb_pd::ucsi::ppm::get_capability::ResponseData as UcsiCapabilities;
use embedded_usb_pd::ucsi::ppm::set_notification_enable::NotificationEnable;
use embedded_usb_pd::ucsi::{Command, ppm};
use log::*;
use static_cell::StaticCell;
use type_c_service::service::config::Config;

#[embassy_executor::task]
async fn task(_spawner: Spawner) {
    embedded_services::init().await;

    controller::init();

    info!("Resetting PPM...");
    let response = execute_ucsi_command(Command::PpmCommand(ppm::Command::PpmReset))
        .await
        .unwrap();
    if !response.response.cci.reset_complete() || response.response.cci.error() {
        error!("PPM reset failed: {:?}", response.response.cci);
    } else {
        info!("PPM reset successful");
    }

    info!("Set Notification enable...");
    let mut notifications = NotificationEnable::default();
    notifications.set_cmd_complete(true);
    let response = execute_ucsi_command(Command::PpmCommand(ppm::Command::SetNotificationEnable(
        ppm::set_notification_enable::Args {
            notification_enable: notifications,
        },
    )))
    .await
    .unwrap();
    if !response.response.cci.cmd_complete() || response.response.cci.error() {
        error!("Set Notification enable failed: {:?}", response.response.cci);
    } else {
        info!("Set Notification enable successful");
    }

    info!("Get PPM capabilities...");
    let response = execute_ucsi_command(Command::PpmCommand(ppm::Command::GetCapability))
        .await
        .unwrap();
    if !response.response.cci.cmd_complete() || response.response.cci.error() {
        error!("Get PPM capabilities failed: {response:?}");
    } else {
        info!("Get PPM capabilities successful: {:?}", response.response.data);
    }
}

fn main() {
    env_logger::builder().filter_level(log::LevelFilter::Trace).init();

    static EXECUTOR: StaticCell<Executor> = StaticCell::new();
    let executor = EXECUTOR.init(Executor::new());
    executor.run(|spawner| {
        spawner.must_spawn(type_c_service::task(Config {
            ucsi_capabilities: UcsiCapabilities {
                num_connectors: 2,
                bcd_usb_pd_spec: 0x0300,
                bcd_type_c_spec: 0x0200,
                bcd_battery_charging_spec: 0x0120,
                ..Default::default()
            },
        }));
        spawner.must_spawn(task(spawner));
    });
}
