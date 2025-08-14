//! Mock eSPI service for the debug-service.
//!
//! This task subscribes to the in-process eSPI channel and logs any received
//! `EspiDebugMessage` chunks. For a `"DATA_READY"` marker, it opportunistically
//! drains the circular buffer into a local staging buffer. In a production system,
//! an actual eSPI task would package and transmit these bytes over OOB.

use defmt::debug;
use embassy_executor::task;
use embassy_sync::channel::Receiver;
use embassy_time::Duration;
use embedded_services::GlobalRawMutex;

use crate::get_buffer_consumer;
use crate::transport::espi::EspiDebugMessage;
use crate::transport::get_debug_channel_receiver;
use heapless::Vec as HeaplessVec;

/// Periodic stats interval
const STATS_INTERVAL: Duration = Duration::from_secs(5);
/// Local staging capacity for accumulating defmt bytes when draining on DATA_READY
const STAGING_CAPACITY: usize = 16 * 1024;

#[task]
pub async fn mock_espi_service() {
    debug!("Mock eSPI service started");
    let rx: Receiver<'static, GlobalRawMutex, EspiDebugMessage, 32> = get_debug_channel_receiver();
    let mut last_stats = embassy_time::Instant::now();
    let mut frames: u32 = 0;
    let mut bytes: u32 = 0;

    // Create a consumer for the debug circular buffer and a local staging buffer
    let framed_consumer = get_buffer_consumer();
    let mut staging: HeaplessVec<u8, STAGING_CAPACITY> = HeaplessVec::new();

    loop {
        let msg = rx.receive().await;

        // Treat a DATA_READY marker as a trigger to drain the circular buffer
        if msg.data.as_slice() == b"DATA_READY" {
            debug!("Mock eSPI: DATA_READY received; draining defmt buffer");

            // Drain all currently available frames without blocking indefinitely
            let mut drained_frames: u32 = 0;
            let mut drained_bytes: u32 = 0;

            // Non-blocking drain of any currently available frames
            while let Ok(frame) = framed_consumer.read() {
                let slice = frame.as_ref();
                let len = slice.len();

                // Attempt to append to staging buffer; if full, drop oldest by clearing
                if staging.capacity() - staging.len() < len {
                    debug!(
                        "Mock eSPI: staging full ({} bytes); clearing to make room",
                        staging.len()
                    );
                    staging.clear();
                }
                let _ = staging.extend_from_slice(slice);

                drained_frames += 1;
                drained_bytes += len as u32;
                frame.release();
            }

            debug!(
                "Mock eSPI: drained {} frames ({} bytes) into staging; staging now {} bytes",
                drained_frames,
                drained_bytes,
                staging.len()
            );
        } else {
            // For any other incoming message, just account/log as before
            frames += 1;
            bytes += msg.data.len() as u32;
            debug!(
                "Mock eSPI OOB: port={} size={} bytes={:?}",
                msg.port,
                msg.data.len(),
                &msg.data[..]
            );
        }

        // Periodic stats
        if embassy_time::Instant::now() - last_stats >= STATS_INTERVAL {
            debug!(
                "Mock eSPI stats: frames={}, bytes={}, staging={} bytes",
                frames,
                bytes,
                staging.len()
            );
            last_stats = embassy_time::Instant::now();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::defmt_ring_logger::enqueue_test_bytes;

    // Helper that performs a single non-blocking drain of the circular buffer into the provided staging vec
    fn single_drain_into(staging: &mut HeaplessVec<u8, STAGING_CAPACITY>) -> (u32, u32) {
        let framed_consumer = get_buffer_consumer();
        let mut drained_frames: u32 = 0;
        let mut drained_bytes: u32 = 0;
        while let Ok(frame) = framed_consumer.read() {
            let slice = frame.as_ref();
            let len = slice.len();
            if staging.capacity() - staging.len() < len {
                staging.clear();
            }
            let _ = staging.extend_from_slice(slice);
            drained_frames += 1;
            drained_bytes += len as u32;
            frame.release();
        }
        (drained_frames, drained_bytes)
    }

    #[test]
    fn drains_defmt_frames_into_staging() {
        // Enqueue some known bytes into the circular buffer
        enqueue_test_bytes(b"espi-frame-1");
        enqueue_test_bytes(b"espi-frame-2-xxxx");

        let mut staging: HeaplessVec<u8, STAGING_CAPACITY> = HeaplessVec::new();
        let (_frames, bytes) = single_drain_into(&mut staging);

        assert!(bytes as usize <= staging.len());
        assert!(staging.len() > 0, "expected staged data");
    }
}
