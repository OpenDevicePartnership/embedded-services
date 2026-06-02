//! Internal observability counters for `embedded-services`.
//!
//! Counters are `AtomicUsize`, free to read at any time. They are bumped
//! from known failure paths so callers can detect regressions of the kinds
//! of bugs that produce silent failures (dropped messages, request
//! overflows, cancelled-command loss, partial broadcasts, poisoned buffers,
//! etc.).
//!
//! The counters are zero-cost when not read: a single atomic increment per
//! bump, no allocation, no formatting. They reset on process restart and do
//! not persist across watchdog resets.
//!
//! # Usage
//!
//! Read a counter at any time:
//!
//! ```
//! use embedded_services::metrics;
//! let _drops = metrics::comms::delegator_errors();
//! ```
//!
//! Use the counters to drive periodic diagnostic dumps or trip a degraded
//! mode when a counter crosses a threshold.

use crate::{AtomicUsize, Ordering};

/// Saturating increment of a counter by `n`. Saturates at `usize::MAX`
/// rather than wrapping so a long-running EC does not silently reset
/// observability state.
#[inline]
fn bump_by(counter: &AtomicUsize, n: usize) {
    // Use compare_exchange_weak loop for saturation. The performance is not
    // a concern on the slow paths these counters are wired to.
    let mut current = counter.load(Ordering::Relaxed);
    loop {
        let next = current.saturating_add(n);
        if next == current {
            // Already saturated.
            return;
        }
        match counter.compare_exchange_weak(current, next, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return,
            Err(actual) => current = actual,
        }
    }
}

/// Counters for the `comms` (IPC routing) layer.
pub mod comms {
    use crate::{AtomicUsize, Ordering};

    static DELEGATOR_ERRORS: AtomicUsize = AtomicUsize::new(0);
    static ROUTED_UNKNOWN: AtomicUsize = AtomicUsize::new(0);
    static DELIVERED_TO_UNINIT: AtomicUsize = AtomicUsize::new(0);

    /// Number of times a registered mailbox delegate returned an error from
    /// `receive` (i.e. dropped a message). The error kind is also `warn!`-
    /// logged at the call site.
    pub fn delegator_errors() -> usize {
        DELEGATOR_ERRORS.load(Ordering::Relaxed)
    }

    /// Number of times a routed message found no matching endpoint at the
    /// destination `EndpointID` (i.e. the destination list contained zero
    /// receivers for the requested id, often because no service has
    /// registered yet).
    pub fn routed_unknown() -> usize {
        ROUTED_UNKNOWN.load(Ordering::Relaxed)
    }

    /// Number of times a routed message reached an `Endpoint` whose delegator
    /// slot was still `None` (i.e. the endpoint had been pushed to its list
    /// but its `init` had not yet completed). Under the single-executor model
    /// this race is impossible because `register_endpoint` runs `push` and
    /// `init` with no yield between them; a non-zero count therefore
    /// indicates either a custom registration path that bypassed
    /// `register_endpoint`, or a multi-executor consumer that violates the
    /// documented model.
    pub fn delivered_to_uninit() -> usize {
        DELIVERED_TO_UNINIT.load(Ordering::Relaxed)
    }

    pub(crate) fn bump_delegator_errors() {
        super::bump_by(&DELEGATOR_ERRORS, 1);
    }

    pub(crate) fn bump_routed_unknown() {
        super::bump_by(&ROUTED_UNKNOWN, 1);
    }

    pub(crate) fn bump_delivered_to_uninit() {
        super::bump_by(&DELIVERED_TO_UNINIT, 1);
    }
}

/// Counters for the `ipc::deferred` request/response channel.
pub mod ipc_deferred {
    use crate::{AtomicUsize, Ordering};

    static DROPPED_COMMANDS: AtomicUsize = AtomicUsize::new(0);
    static ID_MISMATCHES: AtomicUsize = AtomicUsize::new(0);

    /// Number of times a `Channel::execute` future was dropped after
    /// signaling a command but before the responder picked it up, causing
    /// the drop guard to reset the command slot. A non-zero value indicates
    /// caller-side cancellation (timeout, `select`, task abort) of in-flight
    /// requests.
    pub fn dropped_commands() -> usize {
        DROPPED_COMMANDS.load(Ordering::Relaxed)
    }

    /// Number of times `Channel::execute` observed a response whose request
    /// id did not match the caller's id. This is the expected behavior when
    /// a previous caller was cancelled mid-request and the responder later
    /// emits a now-orphaned response.
    pub fn id_mismatches() -> usize {
        ID_MISMATCHES.load(Ordering::Relaxed)
    }

    pub(crate) fn bump_dropped_commands() {
        super::bump_by(&DROPPED_COMMANDS, 1);
    }

    pub(crate) fn bump_id_mismatches() {
        super::bump_by(&ID_MISMATCHES, 1);
    }
}

/// Counters for the `hid` module.
pub mod hid {
    use crate::{AtomicUsize, Ordering};

    static REQUEST_OVERFLOWS: AtomicUsize = AtomicUsize::new(0);
    static DEVICE_REGISTERED_WITHOUT_ENDPOINT: AtomicUsize = AtomicUsize::new(0);

    /// Number of host requests that could not be queued into a
    /// `hid::Device`'s internal channel because the channel was full. The
    /// request is rejected with `MailboxDelegateError::BufferFull` and
    /// counted here.
    pub fn request_overflows() -> usize {
        REQUEST_OVERFLOWS.load(Ordering::Relaxed)
    }

    /// Number of times a `hid::Device` was pushed into the global device
    /// list but its inner `comms::Endpoint` failed to register or the
    /// surrounding future was cancelled before the endpoint registration
    /// completed. The device remains discoverable via `hid::get_device` but
    /// cannot send responses; this counter surfaces the partial-state
    /// hazard that the append-only registry forbids us from cleaning up.
    pub fn device_registered_without_endpoint() -> usize {
        DEVICE_REGISTERED_WITHOUT_ENDPOINT.load(Ordering::Relaxed)
    }

    pub(crate) fn bump_request_overflows() {
        super::bump_by(&REQUEST_OVERFLOWS, 1);
    }

    pub(crate) fn bump_device_registered_without_endpoint() {
        super::bump_by(&DEVICE_REGISTERED_WITHOUT_ENDPOINT, 1);
    }
}

/// Counters for the `broadcaster` module.
pub mod broadcaster {
    use crate::{AtomicUsize, Ordering};

    static LAG: AtomicUsize = AtomicUsize::new(0);

    /// Cumulative count of messages dropped by `DynSubscriber`-based
    /// receivers due to subscriber-side lag. Bumped by the `Lagged(n)` path
    /// in `event::Receiver` implementations.
    ///
    /// Saturates at `usize::MAX` (on 32-bit platforms this is ~4 billion
    /// lagged events).
    pub fn lag() -> usize {
        LAG.load(Ordering::Relaxed)
    }

    pub(crate) fn bump_lag(n: u64) {
        // u64 -> usize: clamp to usize range so 32-bit hosts saturate
        // cleanly rather than truncating large lag values to small ones.
        let n_usize = if n > usize::MAX as u64 { usize::MAX } else { n as usize };
        super::bump_by(&LAG, n_usize);
    }
}

/// Counters for the `buffer` module.
pub mod buffer {
    use crate::{AtomicUsize, Ordering};

    static POISONS: AtomicUsize = AtomicUsize::new(0);

    /// Number of times a `Buffer` transitioned into the terminal
    /// `Poisoned` state (i.e. a borrow guard was dropped while the buffer
    /// was in an unexpected state, indicating a refcount underflow or a
    /// drop of an un-borrowed buffer).
    pub fn poisons() -> usize {
        POISONS.load(Ordering::Relaxed)
    }

    pub(crate) fn bump_poisons() {
        super::bump_by(&POISONS, 1);
    }
}

#[cfg(test)]
mod test {
    use super::*;

    /// `bump_by` saturates at `usize::MAX` rather than wrapping.
    #[test]
    fn test_bump_by_saturates() {
        let counter = AtomicUsize::new(usize::MAX - 3);
        bump_by(&counter, 10);
        assert_eq!(counter.load(Ordering::Relaxed), usize::MAX);

        // Another bump from saturated state is a no-op.
        bump_by(&counter, 5);
        assert_eq!(counter.load(Ordering::Relaxed), usize::MAX);
    }

    /// Bumping by zero is a no-op.
    #[test]
    fn test_bump_by_zero() {
        let counter = AtomicUsize::new(42);
        bump_by(&counter, 0);
        assert_eq!(counter.load(Ordering::Relaxed), 42);
    }

    /// Normal bump increments the value.
    #[test]
    fn test_bump_by_normal() {
        let counter = AtomicUsize::new(10);
        bump_by(&counter, 5);
        assert_eq!(counter.load(Ordering::Relaxed), 15);
    }
}
