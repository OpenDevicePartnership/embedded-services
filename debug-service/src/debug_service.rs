use embassy_sync::{once_lock::OnceLock, signal::Signal};
use embedded_services::GlobalRawMutex;
use embedded_services::comms::{self, EndpointID, Internal};
use embedded_services::debug;
use embedded_services::ec_type::message::{HostMsg, NotificationMsg};

pub struct Service {
    endpoint: comms::Endpoint,
    transport: comms::Endpoint,
}

impl Service {
    pub fn new(endpoint: comms::Endpoint) -> Self {
        Service {
            endpoint: comms::Endpoint::uninit(EndpointID::Internal(Internal::Debug)),
            transport: endpoint,
        }
    }

    pub fn endpoint_id(&self) -> comms::EndpointID {
        self.transport.get_id()
    }
}

impl comms::MailboxDelegate for Service {
    fn receive(&self, message: &comms::Message) -> Result<(), comms::MailboxDelegateError> {
        if let Some(msg) = message.data.get::<HostMsg>() {
            match msg {
                HostMsg::Notification(n) => {
                    // Host acknowledged/triggered; wake the defmt task to respond
                    debug!(
                        "Received host notification (offset={}) from {:?}",
                        n.offset, message.from
                    );
                    notify_signal().signal(*n);
                }
                _ => {
                    debug!("Received host message (non-notification)");
                }
            }
        } else {
            debug!("Got something else");
        }

        Ok(())
    }
}

static DEBUG_SERVICE: OnceLock<Service> = OnceLock::new();

// Global signal used to notify the defmt forwarding task that the Host responded/acknowledged.
static HOST_NOTIFY: OnceLock<Signal<GlobalRawMutex, NotificationMsg>> = OnceLock::new();

/// Get the global notification signal used to synchronize defmt frame responses to the host.
pub fn notify_signal() -> &'static Signal<GlobalRawMutex, NotificationMsg> {
    HOST_NOTIFY.get_or_init(Signal::new)
}

/// Initialize and register the global Debug service endpoint.
///
/// This creates (or reuses) a single [`Service`] instance backed by the
/// provided transport [`comms::Endpoint`], then registers its internal
/// endpoint so messages addressed to [`EndpointID::Internal(Internal::Debug)`]
/// are dispatched to the service's [`comms::MailboxDelegate`] implementation.
///
/// Behavior:
/// - Idempotent: repeated or concurrent calls return the same global instance.
/// - Panics if endpoint registration fails (e.g. duplicate registration).
///
/// The typical caller is the Embassy task [`debug_service`].
///
/// # Example
/// ```no_run
/// use embedded_services::comms;
/// use debug_service::debug_service_entry;
///
/// async fn boot(ep: comms::Endpoint) {
///     debug_service_entry(ep).await;
/// }
/// ```
pub async fn debug_service_entry(endpoint: comms::Endpoint) {
    let debug_service = DEBUG_SERVICE.get_or_init(|| Service::new(endpoint));
    comms::register_endpoint(debug_service, &debug_service.endpoint)
        .await
        .unwrap();
    // Emit an initial defmt frame so the defmt_to_host_task can drain and verify the path.
    debug!("debug service initialized and endpoint registered");
}

#[embassy_executor::task]
pub async fn debug_service(endpoint: comms::Endpoint) {
    debug_service_entry(endpoint).await;
}
