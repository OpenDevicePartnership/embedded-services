use crate::{OdpError, OdpTransport};
use heapless::Vec;

/// In-memory test double for [`OdpTransport`]. Echoes what was sent on
/// the next [`OdpTransport::recv_message`] call.
///
/// Useful for unit testing per-service client logic without standing
/// up a real UART. Capacity is fixed at 256 bytes — large enough for
/// any single ODP message we expect to round-trip in tests.
///
/// # Lifecycle
///
/// - `send_message(payload)` replaces any previously buffered bytes
///   with `payload`.
/// - `recv_message(buf)` copies the buffered bytes into `buf` and
///   clears the internal buffer. A subsequent `recv_message` without
///   an intervening `send_message` returns [`OdpError::Transport`].
pub struct LoopbackTransport {
    queued: Vec<u8, 256>,
}

impl LoopbackTransport {
    pub fn new() -> Self {
        Self { queued: Vec::new() }
    }
}

impl Default for LoopbackTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl OdpTransport for LoopbackTransport {
    fn send_message(&mut self, payload: &[u8]) -> Result<(), OdpError> {
        self.queued.clear();
        self.queued
            .extend_from_slice(payload)
            .map_err(|_| OdpError::BufferTooSmall)?;
        Ok(())
    }

    fn recv_message(&mut self, buf: &mut [u8]) -> Result<usize, OdpError> {
        if self.queued.is_empty() {
            return Err(OdpError::Transport);
        }
        if buf.len() < self.queued.len() {
            return Err(OdpError::BufferTooSmall);
        }
        let n = self.queued.len();
        buf[..n].copy_from_slice(&self.queued);
        self.queued.clear();
        Ok(n)
    }
}
