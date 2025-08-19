/// This example is supposed to init a debug service and a mock eSPI service to demonstrate sending defmt messages from the debug service to the eSPI service
use debug_service::debug_service;
use embassy_executor::{Executor, Spawner};
use embedded_services::comms::{Endpoint, EndpointID, External};
use embedded_services::info;
use static_cell::StaticCell;

// Provide a minimal defmt timestamp for this binary to satisfy defmt's requirement.
// Using 0 disables timestamps in output for simplicity here; adjust as needed.
defmt::timestamp!("{=u64}", { 0u64 });

// Mock eSPI transport service
mod espi_service {
    use embassy_sync::{once_lock::OnceLock, signal::Signal};
    use embedded_services::GlobalRawMutex;
    use embedded_services::comms::{self, EndpointID, External, Internal};
    use embedded_services::ec_type::message::{HostMsg, NotificationMsg};
    use log::info;

    pub struct Service {
        endpoint: comms::Endpoint,
        notify: &'static Signal<GlobalRawMutex, NotificationMsg>,
    }

    impl Service {
        pub fn new(notify: &'static Signal<GlobalRawMutex, NotificationMsg>) -> Self {
            Service {
                endpoint: comms::Endpoint::uninit(EndpointID::External(External::Host)),
                notify,
            }
        }
    }

    impl comms::MailboxDelegate for Service {
        fn receive(&self, message: &comms::Message) -> Result<(), comms::MailboxDelegateError> {
            if let Some(host_msg) = message.data.get::<HostMsg>() {
                match host_msg {
                    HostMsg::Notification(n) => {
                        info!(
                            "mock eSPI got Host Notification: offset={} from {:?}",
                            n.offset, message.from
                        );
                        // Defer sending to async task via signal (receive is not async)
                        self.notify.signal(*n);
                        Ok(())
                    }
                    _ => Err(comms::MailboxDelegateError::MessageNotFound),
                }
            } else {
                Err(comms::MailboxDelegateError::MessageNotFound)
            }
        }
    }

    static ESPI_SERVICE: OnceLock<Service> = OnceLock::new();
    static NOTIFY: OnceLock<Signal<GlobalRawMutex, NotificationMsg>> = OnceLock::new();

    pub async fn init() {
        let notify = NOTIFY.get_or_init(Signal::new);
        let svc = ESPI_SERVICE.get_or_init(|| Service::new(notify));
        comms::register_endpoint(svc, &svc.endpoint).await.unwrap();
    }

    #[embassy_executor::task]
    pub async fn task() {
        loop {
            // Wait for a notification to forward to the debug service
            let svc = ESPI_SERVICE.get().await;
            let n = svc.notify.wait().await;
            info!("mock eSPI: got notification and forwarding to debug service");
            let msg = HostMsg::Notification(n);
            let _ = comms::send(
                EndpointID::External(External::Host),
                EndpointID::Internal(Internal::Debug),
                &msg,
            )
            .await;
        }
    }
}

#[embassy_executor::task]
async fn defmt_frames_task() {
    use embassy_time::{Duration, Timer};
    info!("Hello from defmt frames task");
    loop {
        defmt::info!("Hello from defmt frames task");
        Timer::after(Duration::from_secs(5)).await;
    }
}

#[embassy_executor::task]
async fn init_task(spawner: Spawner) {
    info!("init embedded services");
    embedded_services::init().await;

    info!("init espi service");
    espi_service::init().await;
    spawner.must_spawn(espi_service::task());

    info!("spawn debug service");
    spawner.must_spawn(debug_service(Endpoint::uninit(EndpointID::External(External::Host))));

    info!("spawn defmt_to_host_task");
    spawner.must_spawn(debug_service::defmt_to_host_task());

    spawner.must_spawn(defmt_frames_task());
}

fn main() {
    env_logger::builder().filter_level(log::LevelFilter::Trace).init();

    static EXECUTOR: StaticCell<Executor> = StaticCell::new();
    let executor = EXECUTOR.init(Executor::new());

    executor.run(|spawner| {
        // Spawn debug-service tasks and mock eSPI service
        spawner.must_spawn(init_task(spawner));
    });
}
