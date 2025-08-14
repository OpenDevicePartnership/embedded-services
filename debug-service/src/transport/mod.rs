//! Transport implementations for debug service

use core::future::Future;

/// Transport trait for sending debug data
pub trait DebugTransport {
    /// Send debug frame data
    fn send(&mut self, data: &[u8]) -> impl Future<Output = Result<(), TransportError>>;
}

/// Transport error types
#[derive(Debug)]
pub enum TransportError {
    /// Transport is not available
    Unavailable,
    /// Buffer is full
    BufferFull,
    /// Connection error
    ConnectionError,
    /// Other transport-specific error
    Other(&'static str),
}

pub mod espi;
pub use espi::EspiTransport;
pub use espi::get_debug_channel_receiver;

/// Create the default transport
pub fn create_default_transport() -> impl DebugTransport {
    EspiTransport::new()
}
