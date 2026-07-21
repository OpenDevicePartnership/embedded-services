use embassy_time::Instant;

/// State shared between the port and event receiver
#[derive(Copy, Clone)]
pub struct SharedState {
    /// Sink ready timeout
    pub(crate) sink_ready_deadline: Option<Instant>,
}

impl SharedState {
    /// Create a new instance with default values
    pub fn new() -> Self {
        Self {
            sink_ready_deadline: None,
        }
    }

    /// Get the current sink ready timeout deadline, if one is pending
    pub fn sink_ready_deadline(&self) -> Option<Instant> {
        self.sink_ready_deadline
    }
}

impl Default for SharedState {
    fn default() -> Self {
        Self::new()
    }
}
