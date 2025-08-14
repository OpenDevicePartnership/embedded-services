//! eSPI transport integration for the debug-service.
//!
//! This module exposes a lightweight in-process channel used as the handoff between the
//! debug-service and an eSPI-facing task. The debug-service splits defmt frames into
//! fixed-size OOB payloads and pushes `EspiDebugMessage` instances onto this channel.

use super::{DebugTransport, TransportError};
use core::future::Future;
use embassy_sync::channel::{Channel, Sender};
use embedded_services::GlobalRawMutex;

/// Maximum debug message size for OOB transmission
const MAX_OOB_DEBUG_SIZE: usize = 64;

/// Debug message for eSPI OOB transmission
#[derive(Clone, Debug)]
pub struct EspiDebugMessage {
    /// Debug data to send via OOB
    pub data: heapless::Vec<u8, MAX_OOB_DEBUG_SIZE>,
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
    #[allow(clippy::manual_async_fn)]
    fn send(&mut self, data: &[u8]) -> impl Future<Output = Result<(), TransportError>> {
        async move {
            defmt::debug!("eSPI transport sending {} bytes via OOB", data.len());

            // Split data into OOB-sized chunks and enqueue
            for chunk in data.chunks(MAX_OOB_DEBUG_SIZE) {
                let mut debug_data = heapless::Vec::new();
                if debug_data.extend_from_slice(chunk).is_err() {
                    defmt::error!("Failed to create debug message - chunk too large");
                    return Err(TransportError::ConnectionError);
                }

                let debug_msg = EspiDebugMessage {
                    data: debug_data,
                    port: 0, // Use port 0 for debug messages
                };

                // Use try_send to avoid blocking - if channel is full, drop the message
                match DEBUG_CHANNEL.try_send(debug_msg) {
                    Ok(()) => {
                        defmt::debug!("Debug message queued for OOB transmission");
                    }
                    Err(_) => {
                        defmt::warn!("Debug channel full, dropping message");
                        return Err(TransportError::BufferFull);
                    }
                }
            }

            defmt::debug!("All debug chunks queued successfully");
            Ok(())
        }
    }
}
