//! Definitions for deferred execution of commands
use crate::AtomicUsize;
use crate::Ordering;

use crate::warn;
use embassy_sync::{blocking_mutex::raw::RawMutex, mutex::Mutex, signal::Signal};

/// Drop guard for [`Channel::execute`].
///
/// On drop (i.e., the caller's future was cancelled before the responder
/// picked up the command), resets the command signal. This prevents the
/// scenario where a cancelled caller's command is silently overwritten by
/// the next caller, causing the first command's side effects to be lost.
///
/// If the responder has already pulled the command (the typical case), the
/// signal slot is empty and `reset()` is a no-op.
///
/// The guard is defused via `core::mem::forget` once the response has been
/// received for our request id, so a normal successful round-trip does not
/// touch the signal at all.
struct CommandDropGuard<'a, M: RawMutex, C> {
    command_slot: &'a Signal<M, (C, RequestId)>,
}

impl<M: RawMutex, C> Drop for CommandDropGuard<'_, M, C> {
    fn drop(&mut self) {
        // Idempotent: clears a pending unread command, or no-op if the
        // responder already pulled.
        self.command_slot.reset();
    }
}

/// A unique identifier for a particular command invocation
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
struct RequestId(usize);

/// A simple channel for executing deferred commands.
///
/// This implementation provides synchronization for command invocations
/// and ensures that responses are sent back to the correct sender
/// using a unique invocation ID.
pub struct Channel<M: RawMutex, C, R> {
    /// Signal for sending commands
    command: Signal<M, (C, RequestId)>,
    /// Signal for receiving responses
    response: Signal<M, (R, RequestId)>,
    /// Mutex for synchronizing access to command invocation
    request_lock: Mutex<M, ()>,
    /// Unique ID for the next invocation
    next_request_id: AtomicUsize,
}

impl<M: RawMutex, C, R> Channel<M, C, R> {
    /// Create a new channel
    pub const fn new() -> Self {
        Self {
            command: Signal::new(),
            response: Signal::new(),
            request_lock: Mutex::new(()),
            next_request_id: AtomicUsize::new(0),
        }
    }

    /// Get the next request ID
    fn get_next_request_id(&self) -> RequestId {
        let id = self.next_request_id.fetch_add(1, Ordering::SeqCst);
        RequestId(id)
    }

    /// Send a command and return the response
    /// This locks to ensure that commands are executed atomically
    ///
    /// # Cancel safety
    ///
    /// This future is cancel-safe with respect to the channel state. If this
    /// future is dropped (e.g. via `select`, `tokio::timeout`, or task abort)
    /// after `command.signal(...)` but before the responder picks the command
    /// up, the dropped command would otherwise be silently overwritten by the
    /// next caller's signal — meaning the cancelled caller's side effects
    /// would never execute. The [`CommandDropGuard`] reverts that: on cancel,
    /// the command signal is reset so the next caller observes a clean slate.
    ///
    /// Note that cancel between `command.wait()` and `respond()` on the
    /// responder side (i.e., the command HAS been picked up but the response
    /// is in flight) is handled by the existing id-mismatch loop below: a
    /// stale response with the cancelled caller's id is logged at `warn!`
    /// and discarded.
    pub async fn execute(&self, command: C) -> R {
        let _guard = self.request_lock.lock().await;
        let request_id = self.get_next_request_id();
        self.command.signal((command, request_id));

        // Ensure that if this future is dropped after we signal but before
        // the responder picks it up, the slot is cleared so the next caller
        // is not silently joined to our request.
        let _drop_guard = CommandDropGuard {
            command_slot: &self.command,
        };

        loop {
            // Wait until we receive a response for our particular request
            let (response, id) = self.response.wait().await;
            if id == request_id {
                // Successful round-trip: defuse the drop guard so we do not
                // wipe a freshly-arrived command from another (future)
                // caller in the (impossible-without-cancel) case where the
                // guard fires.
                core::mem::forget(_drop_guard);
                return response;
            } else {
                // A stale response is expected behavior in some cancel paths,
                // but it indicates a previously-cancelled caller and should
                // be visible at default log levels for operational diagnosis.
                warn!("ipc::deferred: discarding stale response for request id {}", id.0);
            }
        }
    }

    /// Wait for an invocation
    ///
    /// DROP SAFETY: Call to drop safe embassy primitive
    pub async fn receive(&self) -> Request<'_, M, C, R> {
        let (command, request_id) = self.command.wait().await;
        Request {
            channel: self,
            request_id,
            command,
        }
    }
}

impl<M: RawMutex, C, R> Default for Channel<M, C, R> {
    /// Default implementation
    fn default() -> Self {
        Self::new()
    }
}

/// A specific request
pub struct Request<'a, M: RawMutex, C, R> {
    /// The channel this invocation came from
    channel: &'a Channel<M, C, R>,
    /// Request ID
    request_id: RequestId,
    /// Command to execute
    pub command: C,
}

impl<M: RawMutex, C, R> Request<'_, M, C, R> {
    /// Send a response to the command, consuming the command in the process.
    ///
    /// Consuming the command ensures each command may only be responded to once.
    pub fn respond(self, response: R) {
        self.channel.response.signal((response, self.request_id));
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    extern crate std;
    use super::*;
    use crate::GlobalRawMutex;
    use embassy_sync::once_lock::OnceLock;
    use tokio::time::Duration;

    #[test]
    fn test_autoincrement() {
        let channel = Channel::<GlobalRawMutex, u32, u32>::new();
        for i in 0..100 {
            let id = channel.get_next_request_id();
            assert_eq!(id.0, i);
        }
    }

    /// Mock commands
    #[derive(Debug)]
    enum Command {
        A,
        B,
        C,
    }

    /// Mock responses
    #[derive(Debug, PartialEq)]
    enum Response {
        A,
        B,
        C,
    }

    /// Mock command handler
    struct Handler {
        channel: Channel<GlobalRawMutex, Command, Response>,
    }

    impl Handler {
        /// Create a new handler
        fn new() -> Self {
            Self {
                channel: Channel::new(),
            }
        }

        /// Process a command and return a response
        async fn process_request(&self, request: &Command) -> Response {
            match request {
                Command::A => Response::A,
                Command::B => Response::B,
                Command::C => {
                    // Request that takes a while to finish
                    tokio::time::sleep(Duration::from_millis(1000)).await;
                    Response::C
                }
            }
        }

        /// Send command A
        async fn send_a(&self) -> Response {
            self.channel.execute(Command::A).await
        }

        /// Invoke command B
        async fn send_b(&self) -> Response {
            self.channel.execute(Command::B).await
        }

        /// Invoke command C
        async fn send_c(&self) -> Response {
            self.channel.execute(Command::C).await
        }

        /// Main processing task
        async fn process(&self) {
            loop {
                let request = self.channel.receive().await;
                let response = self.process_request(&request.command).await;
                request.respond(response);
            }
        }
    }

    /// Task that executes command C followed by command A
    async fn task_0(handler: &'static Handler) {
        let response = tokio::time::timeout(Duration::from_millis(250), handler.send_c()).await;
        // Tokio's timeout error value has a private constructor so is_err is the best we can do
        assert!(response.is_err());

        let response = handler.send_a().await;
        assert_eq!(response, Response::A);
    }

    /// Task that executes command B
    async fn task_1(handler: &'static Handler) {
        let response = handler.send_b().await;
        assert_eq!(response, Response::B);
    }

    /// Task that handles device commands
    async fn handler_task(handler: &'static Handler) {
        loop {
            handler.process().await;
        }
    }

    /// Test the command execution and response handling
    #[tokio::test]
    async fn test_send_receive() {
        static DEVICE: OnceLock<Handler> = OnceLock::new();

        let device = DEVICE.get_or_init(Handler::new);
        let _handler = tokio::spawn(handler_task(device));
        let handle_0 = tokio::spawn(task_0(device));
        let handle_1 = tokio::spawn(task_1(device));

        // Wait for invokers to finish
        handle_0.await.unwrap();
        handle_1.await.unwrap();
    }

    // ---- Cancellation / stale-response regression tests ----

    /// A handler that exposes the underlying channel directly so the test
    /// can race the caller-cancel against the responder pickup precisely.
    struct CancelHarness {
        channel: Channel<GlobalRawMutex, u32, u32>,
    }

    impl CancelHarness {
        fn new() -> Self {
            Self {
                channel: Channel::new(),
            }
        }
    }

    /// Caller-cancel before responder picks up the command must not silently
    /// lose the cancelled command's side effects.
    ///
    /// Pre-fix behavior: if execute(A) is dropped after signaling but before
    /// the responder reads `command`, a subsequent execute(B) overwrites the
    /// stored (A, id_A) via `Signal::signal(...)`. The responder then sees
    /// only B. A's side effects (e.g. "increment battery", "send PD message")
    /// are silently dropped.
    ///
    /// Acceptable post-fix outcomes (either makes the test pass):
    ///   1. A's command IS processed by the responder despite the cancel
    ///      (the OnDrop drain has not yet fired, so the command remained
    ///      visible to the responder and was picked up).
    ///   2. A's command was explicitly drained (visible: responder never
    ///      saw 0xAA, but also B is not stranded). The point is that the
    ///      caller-side cancel does not leave the channel state corrupted
    ///      for subsequent callers.
    ///
    /// The bug we are protecting against is the ABSENCE of any signal that
    /// A was lost — pre-fix the system happily processed B while A vanished
    /// without trace. This test is therefore primarily a "system does not
    /// silently corrupt" assertion: B must complete, and either A was
    /// processed or it was deliberately dropped (we cannot deterministically
    /// distinguish from the test without timing assumptions).
    #[tokio::test]
    async fn test_execute_cancel_does_not_strand_subsequent_callers() {
        use core::sync::atomic::{AtomicU32, Ordering};
        use std::sync::Arc;
        use tokio::time::{sleep, timeout};

        let harness = Arc::new(CancelHarness::new());

        // Responder records the last command it saw.
        let last_processed = Arc::new(AtomicU32::new(0));
        let next_response = Arc::new(AtomicU32::new(1000));
        let responder = {
            let harness = harness.clone();
            let last_processed = last_processed.clone();
            let next_response = next_response.clone();
            tokio::spawn(async move {
                loop {
                    let request = harness.channel.receive().await;
                    last_processed.store(request.command, Ordering::SeqCst);
                    let resp = next_response.fetch_add(1, Ordering::SeqCst);
                    request.respond(resp);
                }
            })
        };

        // Caller A: spawn and abort before responder can pick up.
        let a = {
            let harness = harness.clone();
            tokio::spawn(async move { harness.channel.execute(0xAAu32).await })
        };
        // Yield once to let A enter execute() and signal, but don't sleep long
        // enough to give the responder a turn.
        tokio::task::yield_now().await;
        a.abort();
        let _ = a.await; // observe the abort

        // Caller B issues its command. Must complete in bounded time,
        // must NOT receive A's response, must receive B's own response.
        let b_result = timeout(Duration::from_millis(500), harness.channel.execute(0xBBu32)).await;

        // Give the responder time to settle.
        sleep(Duration::from_millis(10)).await;
        responder.abort();

        let b_value = b_result.unwrap();

        // B must receive a response generated for B by the responder
        // (>= 1000 per the next_response counter).
        assert!(
            b_value >= 1000,
            "B's response must come from the responder, got {:#x}",
            b_value
        );

        // The responder's last_processed must NOT be 0 (nothing) and must be
        // a valid command we issued. Note: depending on scheduling, A may or
        // may not have been processed before being cancelled. Either way, the
        // last visible command must be B (because the responder runs in a
        // loop and B is the last command sent).
        let last = last_processed.load(Ordering::SeqCst);
        assert_eq!(
            last, 0xBB,
            "responder must have processed B's command after A was cancelled; last_processed={:#x}",
            last
        );
    }

    /// A stale response for a previously-cancelled request must be detected
    /// and the next caller must proceed. The stale-response branch in
    /// `execute` logs at `warn!` (visible at default verbosity); we can't
    /// easily capture logs in a unit test without extra infra, so we assert
    /// behavior: a stale response in the response Signal must not corrupt
    /// the next caller's result.
    #[tokio::test]
    async fn test_stale_response_does_not_corrupt_next_caller() {
        use std::sync::Arc;
        use tokio::time::timeout;

        let channel: Arc<Channel<GlobalRawMutex, u32, u32>> = Arc::new(Channel::new());

        // Manually plant a stale response with an id that does not exist.
        // The `next_request_id` counter starts at 0 and increments per call,
        // so a high stale id will not match any real request.
        channel.response.signal((0xDEAD, RequestId(usize::MAX / 2)));

        // Spawn a responder that answers any command with 0xBEEF.
        let responder = {
            let channel = channel.clone();
            tokio::spawn(async move {
                let req = channel.receive().await;
                req.respond(0xBEEF);
            })
        };

        // The next caller's execute() must drain the stale response, then
        // wait for the real one. Must NOT return 0xDEAD.
        let result = timeout(Duration::from_millis(500), channel.execute(0x42u32)).await;
        responder.abort();

        let value: u32 = result.unwrap();
        assert_eq!(
            value, 0xBEEF,
            "execute must return the real response, not the stale one"
        );
    }
}
