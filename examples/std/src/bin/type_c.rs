use embassy_executor::{Executor, Spawner};
use embassy_sync::once_lock::OnceLock;
use embassy_time::Timer;
use embedded_services::type_c::ucsi::lpm;
use embedded_services::type_c::{controller, ControllerId, Error, PortId};
use log::*;
use static_cell::StaticCell;

const CONTROLLER0: ControllerId = ControllerId(0);
const PORT0: PortId = PortId(0);
const PORT1: PortId = PortId(1);

mod test_controller {
    use super::*;

    pub struct Controller<'a> {
        pub controller: controller::Controller<'a>,
    }

    impl controller::ControllerContainer for Controller<'_> {
        fn get_controller(&self) -> &controller::Controller {
            &self.controller
        }
    }

    impl<'a> Controller<'a> {
        pub fn new(id: ControllerId, ports: &'a [PortId]) -> Self {
            Self {
                controller: controller::Controller::new(id, ports),
            }
        }

        async fn process_controller_command(
            &self,
            command: controller::InternalCommandData,
        ) -> Result<controller::InternalResponseData, Error> {
            match command {
                controller::InternalCommandData::Reset => {
                    info!("Reset controller");
                    controller::InternalResponseData::Complete.into()
                }
            }
        }

        async fn process_port_command(&self, command: lpm::Command) -> Result<lpm::ResponseData, Error> {
            match command.operation {
                lpm::CommandData::ConnectorReset(reset_type) => {
                    info!("Reset ({:#?}) for port {:#?}", reset_type, command.port);
                    lpm::ResponseData::Complete.into()
                }
            }
        }

        pub async fn process(&self) {
            let response = match self.controller.wait_command().await {
                controller::Command::Controller(command) => {
                    controller::Response::Controller(self.process_controller_command(command).await)
                }
                controller::Command::Lpm(command) => {
                    controller::Response::Lpm(self.process_port_command(command).await)
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

    let controller = CONTROLLER.get_or_init(|| test_controller::Controller::new(CONTROLLER0, &PORTS));
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

    controller::reset_controller(CONTROLLER0).await.unwrap();
    info!("Reset controller done");
    controller::reset_port(PORT0, lpm::ResetType::Hard).await.unwrap();
    info!("Reset port 0 done");
    controller::reset_port(PORT1, lpm::ResetType::Data).await.unwrap();
    info!("Reset port 1 done");
}

fn main() {
    env_logger::builder().filter_level(log::LevelFilter::Info).init();

    static EXECUTOR: StaticCell<Executor> = StaticCell::new();
    let executor = EXECUTOR.init(Executor::new());
    executor.run(|spawner| {
        spawner.spawn(task(spawner)).unwrap();
    });
}
