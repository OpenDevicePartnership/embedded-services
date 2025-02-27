use embassy_executor::{Executor, Spawner};
use embassy_sync::once_lock::OnceLock;
use embassy_time::Timer;
use embedded_services::power;
use embedded_services::type_c::ucsi::lpm;
use embedded_services::type_c::{controller, ControllerId, GlobalPortId as PortId};
use embedded_usb_pd::PdError as Error;
use log::*;
use static_cell::StaticCell;

const CONTROLLER0: ControllerId = ControllerId(0);
const PORT0: PortId = PortId(0);
const PORT1: PortId = PortId(1);
const POWER0: power::policy::DeviceId = power::policy::DeviceId(0);

mod test_controller {
    use embedded_services::type_c::controller::PortStatus;

    use super::*;

    pub struct Controller {
        pub controller: controller::Device,
        pub power_policy: power::policy::device::Device,
    }

    impl controller::DeviceContainer for Controller {
        fn get_pd_controller_device(&self) -> &controller::Device {
            &self.controller
        }
    }

    impl power::policy::device::DeviceContainer for Controller {
        fn get_power_policy_device(&self) -> &power::policy::device::Device {
            &self.power_policy
        }
    }

    impl Controller {
        pub fn new(id: ControllerId, power_id: power::policy::DeviceId, ports: &[PortId]) -> Result<Self, Error> {
            Ok(Self {
                controller: controller::Device::new(id, ports)?,
                power_policy: power::policy::device::Device::new(power_id),
            })
        }

        async fn process_controller_command(
            &self,
            command: controller::InternalCommandData,
        ) -> Result<controller::InternalResponseData, Error> {
            match command {
                controller::InternalCommandData::Reset => {
                    info!("Reset controller");
                    Ok(controller::InternalResponseData::Complete)
                }
            }
        }

        async fn process_ucsi_command(&self, command: lpm::Command) -> Result<lpm::ResponseData, Error> {
            match command.operation {
                lpm::CommandData::ConnectorReset(reset_type) => {
                    info!("Reset ({:#?}) for port {:#?}", reset_type, command.port);
                    Ok(lpm::ResponseData::Complete)
                }
            }
        }

        async fn process_port_command(
            &self,
            command: controller::PortCommand,
        ) -> Result<controller::PortResponseData, Error> {
            Ok(match command.data {
                controller::PortCommandData::PortStatus => {
                    info!("Port status for port {}", command.port.0);
                    controller::PortResponseData::PortStatus(PortStatus {
                        connection_present: false,
                        contract: None,
                        debug_connection: false,
                    })
                }
                _ => {
                    info!("Port command for port {}", command.port.0);
                    controller::PortResponseData::Complete
                }
            })
        }

        pub async fn process(&self) {
            let response = match self.controller.wait_command().await {
                controller::Command::Controller(command) => {
                    controller::Response::Controller(self.process_controller_command(command).await)
                }
                controller::Command::Lpm(command) => {
                    controller::Response::Lpm(self.process_ucsi_command(command).await)
                }
                controller::Command::Port(command) => {
                    controller::Response::Port(self.process_port_command(command).await)
                }
            };

            self.controller.send_response(response).await
        }
    }
}

#[embassy_executor::task]
async fn controller_task() {
    static CONTROLLER: OnceLock<test_controller::Controller> = OnceLock::new();

    static PORTS: [PortId; 2] = [PORT0, PORT1];

    let controller = CONTROLLER.get_or_init(|| test_controller::Controller::new(CONTROLLER0, POWER0, &PORTS).unwrap());
    controller::register_controller(controller).await.unwrap();

    loop {
        controller.process().await;
    }
}

#[embassy_executor::task]
async fn task(spawner: Spawner) {
    embedded_services::init().await;

    controller::init();

    info!("Starting controller task");
    spawner.must_spawn(controller_task());
    // Wait for controller to be registered
    Timer::after_secs(1).await;

    let context = controller::ContextToken::create().unwrap();

    context.reset_controller(CONTROLLER0).await.unwrap();
    info!("Reset controller done");
    context.reset_port(PORT0, lpm::ResetType::Hard).await.unwrap();
    info!("Reset port 0 done");
    context.reset_port(PORT1, lpm::ResetType::Data).await.unwrap();
    info!("Reset port 1 done");

    let status = context.get_port_status(PORT0).await.unwrap();
    info!("Port 0 status: {:#?}", status);

    let status = context.get_port_status(PORT1).await.unwrap();
    info!("Port 1 status: {:#?}", status);
}

fn main() {
    env_logger::builder().filter_level(log::LevelFilter::Info).init();

    static EXECUTOR: StaticCell<Executor> = StaticCell::new();
    let executor = EXECUTOR.init(Executor::new());
    executor.run(|spawner| {
        spawner.spawn(task(spawner)).unwrap();
    });
}
