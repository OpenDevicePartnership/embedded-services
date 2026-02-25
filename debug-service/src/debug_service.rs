use debug_service_messages::{DebugRequest, DebugResult};
use embassy_sync::{once_lock::OnceLock, signal::Signal};
use embedded_services::GlobalRawMutex;
use embedded_services::buffer::{OwnedRef, SharedRef};
use embedded_services::debug;

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
#[derive(Default)]
pub struct Service {
    // Hack
    frame_available: core::sync::atomic::AtomicBool,
}

impl Service {
    pub const fn new() -> Self {
        Service {
            frame_available: core::sync::atomic::AtomicBool::new(false),
        }
    }
}

impl embedded_services::relay::mctp::RelayServiceHandlerTypes for Service {
    type RequestType = DebugRequest;
    type ResultType = DebugResult;
}

impl embedded_services::relay::mctp::RelayServiceHandler for Service {
    async fn process_request(&self, _request: Self::RequestType) -> Self::ResultType {
        // Host sent an ACPI/MCTP request (e.g. GetDebugBuffer). Treat this as the
        // trigger to send the staged debug buffer back to the host.
        // We only use the signal as a wakeup; the defmt task ignores any payload here.
        if self.frame_available.load(core::sync::atomic::Ordering::SeqCst) {
            response_notify_signal().signal(());
        } else {
            no_avail_notify_signal().signal(());
        }

        frame_ready_signal().wait().await
    }
}

static DEBUG_SERVICE: OnceLock<Service> = OnceLock::new();

// Global signal used to notify tasks waiting on a Host response path (e.g., ACPI response).
// We only need a wake-up, so the payload is unit `()` to avoid lifetime coupling.
static RESP_NOTIFY: OnceLock<Signal<GlobalRawMutex, ()>> = OnceLock::new();

// For no frame avail task
static NO_AVAIL_NOTIFY: OnceLock<Signal<GlobalRawMutex, ()>> = OnceLock::new();

// Frame to send to host
static FRAME_READY: OnceLock<Signal<GlobalRawMutex, DebugResult>> = OnceLock::new();

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

pub(crate) fn frame_ready_signal() -> &'static Signal<GlobalRawMutex, DebugResult> {
    FRAME_READY.get_or_init(Signal::new)
}

/// Initialize and register the global Debug service endpoint.
///
/// This creates (or reuses) a single [`Service`] instance.
///
/// Behavior:
/// - Idempotent: repeated or concurrent calls return the same global instance.
/// - Panics if endpoint registration fails (e.g. duplicate registration).
///
/// The typical caller is the task [`crate::task::debug_service`].
///
/// # Example
/// ```no_run
/// use debug_service::debug_service_entry;
///
/// async fn boot() {
///     debug_service_entry().await;
/// }
/// ```
pub async fn debug_service_entry() {
    let _debug_service = DEBUG_SERVICE.get_or_init(Service::new);
    // Emit an initial defmt frame so the defmt_to_host_task can drain and verify the path.
    debug!("debug service initialized");
}
