use super::*;
use crate::PortEventStreamer;

impl<'a, PSU: Lockable> Service<'a, PSU>
where
    PSU::Inner: psu::Psu,
{
    /// Wait for port flags
    pub(super) async fn wait_port_flags(&self) -> PortEventStreamer {
        if let Some(ref streamer) = self.state.lock().await.port_event_streaming_state {
            // If we have an existing iterator, return it
            // Yield first to prevent starving other tasks
            embassy_futures::yield_now().await;
            *streamer
        } else {
            // Wait for the next port event and create a streamer
            PortEventStreamer::new(self.context.get_unhandled_events().await.into_iter())
        }
    }
}
