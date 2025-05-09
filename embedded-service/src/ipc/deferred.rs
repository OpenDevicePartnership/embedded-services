//! Definitions for deferred execution of commands
use core::{
    marker::PhantomData,
    sync::atomic::{AtomicU32, Ordering},
};

use embassy_sync::{blocking_mutex::raw::RawMutex, mutex::Mutex, signal::Signal};

use crate::debug;

/// A unique identifier for a particular command invocation
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct InvocationId(u32);

/// A simple channel for executing deferred commands
/// This implementation provides synchronization for command invocations
/// and ensures that responses are sent back to the correct invoker
/// using a unique invocation ID.
pub struct Channel<M: RawMutex, C, R> {
    /// Signal for sending commands
    command: Signal<M, (C, InvocationId)>,
    /// Signal for receiving responses
    response: Signal<M, (R, InvocationId)>,
    /// Mutex for synchronizing access to command invocation
    invocation_lock: Mutex<M, ()>,
    /// Unique ID for the next invocation
    next_invocation_id: AtomicU32,
    /// Phantom data for the response type
    _phantom: PhantomData<R>,
}

impl<M: RawMutex, C, R> Channel<M, C, R> {
    /// Create a new channel
    pub const fn new() -> Self {
        Self {
            command: Signal::new(),
            response: Signal::new(),
            invocation_lock: Mutex::new(()),
            _phantom: PhantomData,
            next_invocation_id: AtomicU32::new(0),
        }
    }

    /// Invoke a command and return the response
    /// This locks to ensure that commands are executed atomically
    pub async fn invoke(&self, command: C) -> R {
        let _guard = self.invocation_lock.lock().await;
        let request_id = self.next_invocation_id.fetch_add(1, Ordering::SeqCst);
        let request_id = InvocationId(request_id);
        self.command.signal((command, request_id));
        loop {
            // Wait until we receive a response for out particular invocation
            let (response, id) = self.response.wait().await;
            if id == request_id {
                return response;
            } else {
                // Not an error because this is the expected behavior in certain cases,
                // particularly if the invoker times out before the response is received.
                debug!("Received response for different invocation: {}", id.0);
            }
        }
    }

    /// Wait for an invocation
    pub async fn wait_invocation(&self) -> Invocation<'_, M, C, R> {
        let (command, invocation_id) = self.command.wait().await;
        Invocation {
            channel: self,
            invocation_id,
            command,
        }
    }
}

/// A specific invocation of a command
pub struct Invocation<'a, M: RawMutex, C, R> {
    /// The channel this invocation came from
    channel: &'a Channel<M, C, R>,
    /// Invocation ID
    invocation_id: InvocationId,
    /// Command to execute
    pub command: C,
}

impl<'a, M: RawMutex, C, R> Invocation<'a, M, C, R> {
    /// Send a response to the command
    pub fn send_response(self, response: R) {
        self.channel.response.signal((response, self.invocation_id));
    }
}
