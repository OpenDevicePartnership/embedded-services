use embassy_executor::{Executor, Spawner};
use embassy_sync::once_lock::OnceLock;
use embassy_time::Timer;
use embedded_services::IntrusiveList;
use embedded_usb_pd::ucsi::lpm;
use embedded_usb_pd::{GlobalPortId, PdError as Error};
use log::*;
use static_cell::StaticCell;
use type_c_service::type_c::{Cached, ControllerId, controller};

const CONTROLLER0_ID: ControllerId = ControllerId(0);
const PORT0_ID: GlobalPortId = GlobalPortId(0);
const PORT1_ID: GlobalPortId = GlobalPortId(1);

mod test_controller {
    use embedded_usb_pd::ucsi;
    use type_c_service::type_c::controller::{ControllerStatus, PortStatus};

    use super::*;

    pub struct Controller<'a> {
        pub controller: controller::Device<'a>,
    }

    impl controller::DeviceContainer for Controller<'_> {
        fn get_pd_controller_device(&self) -> &controller::Device<'_> {
            &self.controller
        }
    }

    impl<'a> Controller<'a> {
        pub fn new(id: ControllerId, ports: &'a [GlobalPortId]) -> Self {
            Self {
                controller: controller::Device::new(id, ports),
            }
        }

        async fn process_controller_command(
            &self,
            command: controller::InternalCommandData,
        ) -> Result<controller::InternalResponseData<'static>, Error> {
            match command {
                controller::InternalCommandData::Reset => {
                    info!("Reset controller");
                    Ok(controller::InternalResponseData::Complete)
                }
                controller::InternalCommandData::Status => {
                    info!("Get controller status");
                    Ok(controller::InternalResponseData::Status(ControllerStatus {
                        mode: "Test",
                        valid_fw_bank: true,
                        fw_version0: 0xbadf00d,
                        fw_version1: 0xdeadbeef,
                    }))
                }
                controller::InternalCommandData::SyncState => {
                    info!("Sync controller state");
                    Ok(controller::InternalResponseData::Complete)
                }
            }
        }

        async fn process_ucsi_command(&self, command: &lpm::GlobalCommand) -> ucsi::GlobalResponse {
            match command.operation() {
                lpm::CommandData::ConnectorReset => {
                    info!("Reset for port {:#?}", command.port());
                    ucsi::Response {
                        cci: ucsi::cci::Cci::new_cmd_complete(),
                        data: None,
                    }
                }
                rest => {
                    info!("UCSI command {:#?} for port {:#?}", rest, command.port());
                    ucsi::Response {
                        cci: ucsi::cci::Cci::new_cmd_complete(),
                        data: None,
                    }
                }
            }
        }

        async fn process_port_command(
            &self,
            command: controller::PortCommand,
        ) -> Result<controller::PortResponseData, Error> {
            Ok(match command.data {
                controller::PortCommandData::PortStatus(Cached(true)) => {
                    info!("Port status for port {}", command.port.0);
                    controller::PortResponseData::PortStatus(PortStatus::new())
                }
                _ => {
                    info!("Port command for port {}", command.port.0);
                    controller::PortResponseData::Complete
                }
            })
        }

        pub async fn process(&self) {
            let request = self.controller.receive().await;
            let response = match request.command {
                controller::Command::Controller(command) => {
                    controller::Response::Controller(self.process_controller_command(command).await)
                }
                controller::Command::Lpm(command) => {
                    controller::Response::Ucsi(self.process_ucsi_command(&command).await)
                }
                controller::Command::Port(command) => {
                    controller::Response::Port(self.process_port_command(command).await)
                }
            };

            request.respond(response);
        }
    }
}

#[embassy_executor::task]
async fn controller_task(controller_list: &'static IntrusiveList) {
    static CONTROLLER: OnceLock<test_controller::Controller> = OnceLock::new();

    static PORTS: [GlobalPortId; 2] = [PORT0_ID, PORT1_ID];

    let controller = CONTROLLER.get_or_init(|| test_controller::Controller::new(CONTROLLER0_ID, &PORTS));
    controller::register_controller(controller_list, controller).unwrap();

    loop {
        controller.process().await;
    }
}

#[embassy_executor::task]
async fn task(spawner: Spawner) {
    embedded_services::init().await;

    static CONTROLLER_LIST: StaticCell<IntrusiveList> = StaticCell::new();
    let controller_list = CONTROLLER_LIST.init(IntrusiveList::new());

    info!("Starting controller task");
    spawner.must_spawn(controller_task(controller_list));
    // Wait for controller to be registered
    Timer::after_secs(1).await;

    let context = controller::Context::new();

    context.reset_controller(controller_list, CONTROLLER0_ID).await.unwrap();

    let status = context
        .get_controller_status(controller_list, CONTROLLER0_ID)
        .await
        .unwrap();
    info!("Controller 0 status: {status:#?}");

    let status = context
        .get_port_status(controller_list, PORT0_ID, Cached(true))
        .await
        .unwrap();
    info!("Port 0 status: {status:#?}");

    let status = context
        .get_port_status(controller_list, PORT1_ID, Cached(true))
        .await
        .unwrap();
    info!("Port 1 status: {status:#?}");
}

fn main() {
    env_logger::builder().filter_level(log::LevelFilter::Info).init();

    static EXECUTOR: StaticCell<Executor> = StaticCell::new();
    let executor = EXECUTOR.init(Executor::new());
    executor.run(|spawner| {
        spawner.spawn(task(spawner)).unwrap();
    });
}
