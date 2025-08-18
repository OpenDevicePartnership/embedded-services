//! Definitions for deferred execution of commands
use core::sync::atomic::{AtomicUsize, Ordering};

use crate::debug;
use embassy_sync::{blocking_mutex::raw::RawMutex, mutex::Mutex, signal::Signal};

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
    pub async fn execute(&self, command: C) -> R {
        let _guard = self.request_lock.lock().await;
        let request_id = self.get_next_request_id();
        self.command.signal((command, request_id));
        loop {
            // Wait until we receive a response for out particular request
            let (response, id) = self.response.wait().await;
            if id == request_id {
                return response;
            } else {
                // Not an error because this is the expected behavior in certain cases,
                // particularly if the sender times out before the response is received.
                debug!("Received response for different invocation: {}", id.0);
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
mod tests {
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
}
