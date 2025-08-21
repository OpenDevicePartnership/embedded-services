use embassy_sync::{once_lock::OnceLock, signal::Signal};
use embedded_services::GlobalRawMutex;
use embedded_services::comms::{self, EndpointID, Internal};
use embedded_services::ec_type::message::{AcpiMsgComms, HostMsg, NotificationMsg};
use embedded_services::{debug, error};

/// Debug service that bridges an internal endpoint to an external transport.
///
/// Terminology:
/// - Transport: The external-facing `comms::Endpoint` used to reach the host/PC.
///   It is provided by the platform (eSPI, USB, RTT bridge, etc.) and passed to
///   [`Service::new`]. Its ID is commonly `EndpointID::External(External::Host)`,
///   but the service does not assume a specific value.
/// - Endpoint: The internal endpoint owned by this service and registered under
///   `EndpointID::Internal(Internal::Debug)`. Messages addressed to this ID are
///   dispatched to the service via [`comms::MailboxDelegate::receive`].
///
/// Direction:
/// - Device → Host: Producers (e.g., the defmt forwarding task) should send from
///   `EndpointID::Internal(Internal::Debug)` to the transport endpoint ID exposed
///   by [`Service::endpoint_id`] or [`host_endpoint_id`].
/// - Host → Device: The platform transport should deliver host messages to
///   `EndpointID::Internal(Internal::Debug)`, which this service handles in
///   [`receive`](comms::MailboxDelegate::receive).
pub struct Service {
    // The service-owned internal endpoint (Internal::Debug) that is registered
    // with the comms layer and used as the "device side" address.
    endpoint: comms::Endpoint,
    // The external transport endpoint through which host traffic flows.
    // This is provided by the platform and may map to eSPI/USB/etc.
    transport: comms::Endpoint,
}

impl Service {
    pub fn new(endpoint: comms::Endpoint) -> Self {
        Service {
            endpoint: comms::Endpoint::uninit(EndpointID::Internal(Internal::Debug)),
            transport: endpoint,
        }
    }

    /// Returns the `EndpointID` of the external transport used by this service.
    ///
    /// Other components should target this ID when sending messages to the host
    /// via the debug transport.
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
        } else if let Some(acpi) = message.data.get::<AcpiMsgComms>() {
            // Host sent an ACPI/MCTP request (e.g. GetDebugBuffer). Treat this as the
            // trigger to send the staged debug buffer back to the host.
            debug!(
                "Received host ACPI request for debug buffer (len={}) from {:?}",
                acpi.payload_len, message.from
            );
            // We only use the signal as a wakeup; the defmt task ignores the offset value.
            notify_signal().signal(NotificationMsg { offset: 20 });
        } else {
            error!("Received unknown message from host");
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

/// Returns the endpoint ID of the transport used by the debug service.
pub async fn host_endpoint_id() -> EndpointID {
    let svc = DEBUG_SERVICE.get().await;
    svc.endpoint_id()
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
