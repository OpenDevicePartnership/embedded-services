use embassy_sync::{once_lock::OnceLock, signal::Signal};
use embedded_services::GlobalRawMutex;
use embedded_services::buffer::{OwnedRef, SharedRef};
use embedded_services::comms::{self, EndpointID, Internal};
use embedded_services::{debug, error};

// Maximum number of bytes to request per defmt frame write grant.
// This decouples the logger from any external protocol-specific size constants.
// Each BBQueue "frame" is created by requesting a framed write grant of at most
// DEFMT_MAX_BYTES and then committing it atomically. A commit publishes the frame
// in one shot to the consumer; there is no concept of a "partial" frame being
// visible. This guarantees:
// - Per-frame upper bound: at most 1024 bytes are ever committed in a single frame.
// - No partial publication: a frame is either not yet committed or fully committed.
// If a defmt log event were to exceed this size, it will be split across multiple
// BBQueue frames (each ≤ 1024). The consumer always observes complete frames.
pub(crate) const DEFMT_MAX_BYTES: u16 = debug_service_messages::STD_DEBUG_BUF_SIZE as u16;

// Static buffer for ACPI-style messages carrying defmt frames
embedded_services::define_static_buffer!(defmt_acpi_buf, u8, [0u8; DEFMT_MAX_BYTES as usize]);

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
    // Hack
    frame_available: core::sync::atomic::AtomicBool,
}

impl Service {
    pub const fn new(endpoint: comms::Endpoint) -> Self {
        Service {
            endpoint: comms::Endpoint::uninit(EndpointID::Internal(Internal::Debug)),
            transport: endpoint,
            frame_available: core::sync::atomic::AtomicBool::new(false),
        }
    }

    /// Returns the `EndpointID` of the external transport used by this service.
    ///
    /// Other components should target this ID when sending messages to the host
    /// via the debug service
    pub fn endpoint_id(&self) -> comms::EndpointID {
        self.transport.get_id()
    }
}

impl comms::MailboxDelegate for Service {
    fn receive(&self, message: &comms::Message) -> Result<(), comms::MailboxDelegateError> {
        if let Some(_request) = message.data.get::<debug_service_messages::DebugRequest>() {
            // Host sent an ACPI/MCTP request (e.g. GetDebugBuffer). Treat this as the
            // trigger to send the staged debug buffer back to the host.
            embedded_services::trace!("Received host ACPI request for debug buffer from {:?}", message.from);
            // We only use the signal as a wakeup; the defmt task ignores any payload here.
            if self.frame_available.load(core::sync::atomic::Ordering::SeqCst) {
                response_notify_signal().signal(());
            } else {
                no_avail_notify_signal().signal(());
            }
        } else {
            error!("Received unknown message from host");
        }

        Ok(())
    }
}

static DEBUG_SERVICE: OnceLock<Service> = OnceLock::new();

// Global signal used to notify tasks waiting on a Host response path (e.g., ACPI response).
// We only need a wake-up, so the payload is unit `()` to avoid lifetime coupling.
static RESP_NOTIFY: OnceLock<Signal<GlobalRawMutex, ()>> = OnceLock::new();

// For no frame avail task
static NO_AVAIL_NOTIFY: OnceLock<Signal<GlobalRawMutex, ()>> = OnceLock::new();

pub(crate) fn owned_buffer() -> OwnedRef<'static, u8> {
    defmt_acpi_buf::get_mut().expect("defmt staging buffer already initialized elsewhere")
}

pub(crate) fn shared_buffer() -> SharedRef<'static, u8> {
    defmt_acpi_buf::get()
}

pub(crate) fn frame_available(avail: bool) {
    let s = DEBUG_SERVICE.try_get().expect("Debug service must be init");
    s.frame_available.store(avail, core::sync::atomic::Ordering::SeqCst);
}

pub(crate) fn response_notify_signal() -> &'static Signal<GlobalRawMutex, ()> {
    RESP_NOTIFY.get_or_init(Signal::new)
}

pub(crate) fn no_avail_notify_signal() -> &'static Signal<GlobalRawMutex, ()> {
    NO_AVAIL_NOTIFY.get_or_init(Signal::new)
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
/// The typical caller is the task [`crate::task::debug_service`].
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
