//! Thermal service
#![no_std]

pub use context::*;
use default_handler::DefaultHandler;
use embassy_sync::once_lock::OnceLock;
use embedded_services::{comms, error, info};

mod context;
pub mod default_handler;
pub mod fan;
pub mod mptf;
pub mod sensor;
pub mod utils;

/// Request to the thermal service wrapping an MPTF request and sender ID
#[derive(Debug, Clone, Copy)]
pub struct Request {
    pub mptf: mptf::Request,
    pub from: Option<comms::EndpointID>,
}

impl Request {
    /// Create new thermal service request
    pub fn new(request: mptf::Request, from: Option<comms::EndpointID>) -> Self {
        Self { mptf: request, from }
    }
}

struct Service<M: mptf::Handler + 'static> {
    context: context::ContextToken,
    endpoint: comms::Endpoint,
    mptf_handler: &'static M,
}

impl<M: mptf::Handler> Service<M> {
    fn new(mptf_handler: &'static M) -> Option<Self> {
        Some(Self {
            context: context::ContextToken::create()?,
            endpoint: comms::Endpoint::uninit(comms::EndpointID::Internal(comms::Internal::Thermal)),
            mptf_handler,
        })
    }

    async fn process_request(&self, request: mptf::Request) -> mptf::Response {
        match request {
            mptf::Request::GetTmp(tzid) => self.mptf_handler.get_tmp(tzid).await,
            mptf::Request::GetThrs(tzid) => self.mptf_handler.get_thrs(tzid).await,
            mptf::Request::SetThrs(tzid, timeout, low, high) => {
                self.mptf_handler.set_thrs(tzid, timeout, low, high).await
            }
            mptf::Request::SetScp(tzid, mode, acoustic_lim, power_lim) => {
                self.mptf_handler.set_scp(tzid, mode, acoustic_lim, power_lim).await
            }
            mptf::Request::GetVar(uid) => self.mptf_handler.get_var(uid).await,
            mptf::Request::SetVar(uid, val) => self.mptf_handler.set_var(uid, val).await,
        }
    }

    async fn wait_and_process(&self) {
        let request = self.context.wait_request().await;
        let response = self.process_request(request.command.mptf).await;
        request.respond(response);
    }

    async fn wait_and_process_comms(&self) {
        let request = self.context.wait_comms_request().await;

        // Essentially convert a sync comms request into an async IPC request
        let response = context::execute_service_request(request).await;

        // And then route the response back to the service that made the request
        let original_sender = request.from.expect("Request guaranteed to have from field");
        send_service_msg(original_sender, &response).await
    }
}

impl<M: mptf::Handler> comms::MailboxDelegate for Service<M> {
    fn receive(&self, message: &comms::Message) -> Result<(), comms::MailboxDelegateError> {
        // If standard MPTF request, send to comms channel for later async processing
        if let Some(&msg) = message.data.get::<mptf::Request>() {
            self.context
                .send_request_from_comms(Request {
                    mptf: msg,
                    from: Some(message.from),
                })
                .map_err(|_| comms::MailboxDelegateError::BufferFull)
        } else {
            Err(comms::MailboxDelegateError::InvalidData)
        }
    }
}

// Just one instance of the service should be running
// TODO: Use Macro to make this all generic over MPTF Handler and task
static SERVICE: OnceLock<Service<DefaultHandler>> = OnceLock::new();

/// This must be called to initialize the Thermal service and spawn additional tasks
pub async fn init(spawner: embassy_executor::Spawner, handler: &'static DefaultHandler) {
    info!("Starting thermal service task");
    let service = SERVICE.get_or_init(|| Service::new(handler).expect("Thermal service singleton already initialized"));

    if comms::register_endpoint(service, &service.endpoint).await.is_err() {
        error!("Failed to register thermal service endpoint");
        return;
    }

    // Spawn the tasks for receiving thermal messages
    spawner.must_spawn(rx_task());
    spawner.must_spawn(comms_rx_task());
}

/// Used to send messages to other services from the Thermal service,
/// such as notifying the Host of thresholds crossed or the Power service if CRT TEMP is reached.
pub async fn send_service_msg(to: comms::EndpointID, data: &impl embedded_services::Any) {
    let service = SERVICE.get().await;

    // TODO: When this gets updated to return error, handle retrying send on failure
    service.endpoint.send(to, data).await.expect("Infallible");
}

// This task exists solely to listen for incoming requests and then process them appropriately
#[embassy_executor::task]
async fn rx_task() {
    let service = SERVICE.get().await;

    loop {
        service.wait_and_process().await;
    }
}

// This task exists solely to listen for incoming requests from comms service and then process them appropriately
#[embassy_executor::task]
async fn comms_rx_task() {
    let service = SERVICE.get().await;

    loop {
        service.wait_and_process_comms().await;
    }
}
