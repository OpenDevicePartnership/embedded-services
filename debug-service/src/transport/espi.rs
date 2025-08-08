//! eSPI transport implementation

use super::{DebugTransport, TransportError};
use core::future::Future;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::{Channel, Sender};

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

/// Global channel for sending debug messages to eSPI service
/// This channel allows the debug service to queue messages for OOB transmission
/// Increased capacity to handle burst traffic better
static DEBUG_CHANNEL: Channel<CriticalSectionRawMutex, EspiDebugMessage, 32> = Channel::new();

/// Get a sender handle for the debug channel
/// This should be called by the eSPI service to get a receiver
pub fn get_debug_channel_sender() -> Sender<'static, CriticalSectionRawMutex, EspiDebugMessage, 32> {
    DEBUG_CHANNEL.sender()
}

/// Get a receiver handle for the debug channel  
/// This should be called by the eSPI service to receive debug messages
pub fn get_debug_channel_receiver()
-> embassy_sync::channel::Receiver<'static, CriticalSectionRawMutex, EspiDebugMessage, 32> {
    DEBUG_CHANNEL.receiver()
}

/// eSPI transport implementation
pub struct EspiTransport {
    // Add fields for eSPI transport configuration if needed
    // For example: channel, buffer size, etc.
}

impl EspiTransport {
    /// Create a new eSPI transport
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

            // Split data into chunks that fit in OOB messages
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
