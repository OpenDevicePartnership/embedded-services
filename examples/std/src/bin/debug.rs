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
    use embedded_services::buffer::OwnedRef;
    use embedded_services::GlobalRawMutex;
    use embedded_services::comms::{self, EndpointID, External};
    use embedded_services::ec_type::message::{HostMsg, NotificationMsg};
    use log::{info, trace};

    // Max defmt payload we expect to shuttle in this mock
    const MAX_DEFMT_BYTES: usize = 1024;
    embedded_services::define_static_buffer!(host_oob_buf, u8, [0u8; MAX_DEFMT_BYTES]);

    pub struct Service {
        endpoint: comms::Endpoint,
        notify: &'static Signal<GlobalRawMutex, NotificationMsg>,
        // Signal to wake the host when a response payload has been staged
        resp_len: &'static Signal<GlobalRawMutex, usize>,
        // Owned access so we can stage the response bytes for the host to read
        resp_owned: OwnedRef<'static, u8>,
    }

    impl Service {
        pub fn new(notify: &'static Signal<GlobalRawMutex, NotificationMsg>) -> Self {
            Service {
                endpoint: comms::Endpoint::uninit(EndpointID::External(External::Host)),
                notify,
                resp_len: RESP_LEN.get_or_init(Signal::new),
                resp_owned: host_oob_buf::get_mut().unwrap(),
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
                        // Defer to async host task via signal (receive is not async)
                        self.notify.signal(*n);
                        Ok(())
                    }
                    HostMsg::Response(acpi) => {
                        // Stage the response bytes into the mock OOB buffer for the host
                        let mut access = self.resp_owned.borrow_mut();
                        let buf: &mut [u8] = core::borrow::BorrowMut::borrow_mut(&mut access);
                        let src = acpi.payload.borrow();
                        let src_slice: &[u8] = core::borrow::Borrow::borrow(&src);
                        let copy_len = core::cmp::min(acpi.payload_len, buf.len());
                        buf[..copy_len].copy_from_slice(&src_slice[..copy_len]);
                        trace!("mock eSPI staged {copy_len} response bytes for host");
                        self.resp_len.signal(copy_len);
                        Ok(())
                    }
                }
            } else {
                Err(comms::MailboxDelegateError::MessageNotFound)
            }
        }
    }

    static ESPI_SERVICE: OnceLock<Service> = OnceLock::new();
    static NOTIFY: OnceLock<Signal<GlobalRawMutex, NotificationMsg>> = OnceLock::new();
    static RESP_LEN: OnceLock<Signal<GlobalRawMutex, usize>> = OnceLock::new();

    pub async fn init() {
        let notify = NOTIFY.get_or_init(Signal::new);
        let svc = ESPI_SERVICE.get_or_init(|| Service::new(notify));
        comms::register_endpoint(svc, &svc.endpoint).await.unwrap();
    }

    // Expose signals/buffer to the mock host
    pub async fn wait_host_notification() -> NotificationMsg {
        let svc = ESPI_SERVICE.get().await;
        svc.notify.wait().await
    }

    pub async fn wait_response_len() -> usize {
        let svc = ESPI_SERVICE.get().await;
        svc.resp_len.wait().await
    }

    pub fn response_buf() -> embedded_services::buffer::SharedRef<'static, u8> {
        host_oob_buf::get()
    }
}

// Mock Host (eSPI master)
mod host {
    use embedded_services::comms::{self, EndpointID, External, Internal};
    use embedded_services::ec_type::message::{AcpiMsgComms, NotificationMsg};
    use core::borrow::BorrowMut;
    use log::info;

    #[embassy_executor::task]
    pub async fn task() {
        // Static request buffer used to build the "GetDebugBuffer" payload
        embedded_services::define_static_buffer!(debug_req_buf, u8, [0u8; 32]);
    // Acquire owned access once; subsequent get_mut() calls would return None
    let req_owned = debug_req_buf::get_mut().unwrap();

        loop {
            // Wait for a device notification via the mock eSPI transport
            let n: NotificationMsg = super::espi_service::wait_host_notification().await;
            info!(
                "Host: got notification (offset={}), sending OOB request/ACK to Debug",
                n.offset
            );

            // Build the ACPI/MCTP-style request payload for the Debug service
            let request = b"GetDebugBuffer";
            let req_len = request.len();
            {
                let mut access = req_owned.borrow_mut();
                let buf: &mut [u8] = BorrowMut::borrow_mut(&mut access);
                buf[..req_len].copy_from_slice(request);
            }

            // Send an ACK/"OOB request" (as AcpiMsgComms) to the Debug service
            let _ = comms::send(
                EndpointID::External(External::Host),
                EndpointID::Internal(Internal::Debug),
                &AcpiMsgComms {
                    payload: debug_req_buf::get(),
                    payload_len: req_len,
                },
            )
            .await;

            // Wait for the response payload staged by the mock eSPI transport
            let len = super::espi_service::wait_response_len().await;
            let buf = super::espi_service::response_buf();
            let access = buf.borrow();
            let slice: &[u8] = core::borrow::Borrow::borrow(&access);
            let bytes = &slice[..len.min(slice.len())];
            // Print a compact hex dump of the first few bytes
            let preview = bytes.iter().take(32).map(|b| format!("{b:02X}")).collect::<Vec<_>>()
                .join(" ");
            info!("Host: got OOB response ({len} bytes). First 32: {preview}");
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
    // Spawn mock Host (eSPI master) to drive the OOB request/response flow
    spawner.must_spawn(host::task());

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
