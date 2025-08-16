//! eSPI transport integration for the debug-service.
//!
//! This module exposes a lightweight in-process channel used as the handoff between the
//! debug-service and an eSPI-facing task. The debug-service splits defmt frames into
//! fixed-size OOB payloads and pushes `EspiDebugMessage` instances onto this channel.

use super::{DebugTransport, TransportError};
use core::future::Future;
use embassy_sync::channel::{Channel, Sender};
use embedded_services::GlobalRawMutex;

/// Maximum debug frame size to forward in a single message (no chunking).
/// Keep in sync with the logger's maximum frame size.
pub const MAX_DEBUG_FRAME_SIZE: usize = 1024;

/// Debug message for eSPI OOB transmission
#[derive(Clone, Debug)]
pub struct EspiDebugMessage {
    /// Debug data to send via OOB
    pub data: heapless::Vec<u8, MAX_DEBUG_FRAME_SIZE>,
    /// Port number for OOB transmission (0 for debug)
    pub port: u8,
}

/// Global channel for handing off debug messages to an eSPI task.
/// Increased capacity helps absorb bursts of defmt traffic.
/// Uses `GlobalRawMutex` (ThreadMode on target, CriticalSection on host/tests).
static DEBUG_CHANNEL: Channel<GlobalRawMutex, EspiDebugMessage, 32> = Channel::new();

/// Get a sender handle for the debug channel
/// This should be called by the eSPI service to get a receiver
pub fn get_debug_channel_sender() -> Sender<'static, GlobalRawMutex, EspiDebugMessage, 32> {
    DEBUG_CHANNEL.sender()
}

/// Get a receiver handle for the debug channel
/// This should be called by the eSPI service to receive debug messages
pub fn get_debug_channel_receiver() -> embassy_sync::channel::Receiver<'static, GlobalRawMutex, EspiDebugMessage, 32> {
    DEBUG_CHANNEL.receiver()
}

/// eSPI transport implementation
pub struct EspiTransport {
    // Add fields for eSPI transport configuration if needed
    // For example: channel, buffer size, etc.
}

impl EspiTransport {
    /// Create a new eSPI transport used by the send task
    pub fn new() -> Self {
        Self {}
    }

    /// Create eSPI transport with custom configuration
    pub fn with_config(/* config parameters */) -> Self {
        Self {}
    }
}

impl Default for EspiTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl DebugTransport for EspiTransport {
    fn send(&mut self, data: &[u8]) -> impl Future<Output = Result<(), TransportError>> {
        // Copy into an owned vector before the async block to avoid borrowing across await
        let mut frame: heapless::Vec<u8, MAX_DEBUG_FRAME_SIZE> = heapless::Vec::new();
        let too_large = frame.extend_from_slice(data).is_err();
        let len = data.len();

        async move {
            if too_large {
                defmt::warn!("Debug frame too large for single-message handoff ({} bytes)", len);
                return Err(TransportError::BufferFull);
            }

            defmt::debug!("eSPI transport enqueuing {}-byte frame without chunking", len);
            let debug_msg = EspiDebugMessage { data: frame, port: 0 };

            match DEBUG_CHANNEL.try_send(debug_msg) {
                Ok(()) => {
                    defmt::debug!("Debug frame queued for OOB service");
                    Ok(())
                }
                Err(_) => {
                    defmt::warn!("Debug channel full, dropping frame");
                    Err(TransportError::BufferFull)
                }
            }
        }
    }
}
