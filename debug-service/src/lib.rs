//! Debug service for embedded systems that integrates with defmt logging.
//!
//! This service provides a circular buffer that captures defmt log output
//! for later retrieval, useful for debugging embedded systems where direct
//! console output may not be available.
//!
//! ## Testing
//!
//! Due to the use of global static state, tests should be run with a single thread:
//! ```bash
//! cargo test -- --test-threads=1
//! ```
#![no_std]

use core::{
    marker::Sized,
    ops::{Deref, DerefMut},
    option::Option::{self, None, Some},
    ptr::addr_of_mut,
    result::Result::Err,
    sync::atomic::{AtomicBool, Ordering},
};

use bbq2::{
    prod_cons::framed::{FramedGrantW, FramedProducer},
    queue::BBQueue,
    traits::{coordination::cas::AtomicCoord, notifier::maitake::MaiNotSpsc, storage::Inline},
};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};

// Export transport module and types
pub mod transport;
pub use transport::get_debug_channel_receiver;
pub use transport::{DebugTransport, TransportError};

static RTT_INITIALIZED: AtomicBool = AtomicBool::new(false);
static mut ENCODER: defmt::Encoder = defmt::Encoder::new();
static mut RESTORE_STATE: critical_section::RestoreState = critical_section::RestoreState::invalid();

// Debug counter to track how many writes go through our custom logger
static WRITE_COUNT: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

// Debug counter to track how many frames are completed
static FRAME_COUNT: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

// Buffer size for the circular buffer (adjustable)
const BUFFER_SIZE: usize = 4096;
// Maximum bytes per defmt frame
const DEFMT_MAX_BYTES: u16 = 1024;

type Queue = BBQueue<Inline<BUFFER_SIZE>, AtomicCoord, MaiNotSpsc>;

// Buffer size for RTT channel
const RTT_BUFFER_SIZE: usize = 4096;

static DEFMT_BUFFER: Queue = Queue::new();
static mut WRITE_GRANT: Option<FramedGrantW<&'static Queue>> = None;
static mut WRITTEN: usize = 0;

// Signal to notify external services (e.g. eSPI service) that newly encoded
// defmt data has been committed to the circular buffer and is ready to be
// consumed. Future work will gate signalling on a configurable threshold; for
// now we signal on every commit that writes >0 bytes.
static DATA_AVAILABLE_SIGNAL: Signal<CriticalSectionRawMutex, ()> = Signal::new();

/// Safety:
/// Only one producer reference may exist at one time
unsafe fn get_producer() -> &'static mut FramedProducer<&'static Queue> {
    static mut PRODUCER: Option<FramedProducer<&'static Queue>> = None;

    let producer = unsafe { &mut *addr_of_mut!(PRODUCER) };

    match producer {
        Some(p) => p,
        None => producer.insert(DEFMT_BUFFER.framed_producer()),
    }
}

/// Safety:
/// Only one grant reference may exist at one time
unsafe fn get_write_grant() -> Option<(&'static mut [u8], &'static mut usize)> {
    let write_grant = unsafe { &mut *addr_of_mut!(WRITE_GRANT) };

    let write_grant = match write_grant {
        Some(wg) => wg,
        wg @ None => wg.insert(unsafe { get_producer() }.grant(DEFMT_MAX_BYTES).ok()?),
    };

    Some((write_grant.deref_mut(), unsafe { &mut *addr_of_mut!(WRITTEN) }))
}

unsafe fn commit_write_grant() {
    // Capture how many bytes were written before we reset state so we can
    // decide whether to fire the data-available signal.
    let written_bytes = unsafe { WRITTEN };

    if let Some(wg) = unsafe { &mut *addr_of_mut!(WRITE_GRANT) }.take() {
        wg.commit(written_bytes as u16)
    }

    // Reset immediately so further writes start fresh
    unsafe {
        WRITTEN = 0;
    }

    // Only signal if we actually committed data. This keeps noise down while
    // still enabling a simple "data ready" wakeup for the consumer side.
    if written_bytes > 0 {
        DATA_AVAILABLE_SIGNAL.signal(());
    }
}

#[defmt::global_logger]
struct DefmtLogger;
unsafe impl defmt::Logger for DefmtLogger {
    fn acquire() {
        unsafe { RESTORE_STATE = critical_section::acquire() }
        unsafe {
            (&mut *addr_of_mut!(ENCODER)).start_frame(|bytes| write(bytes));
        }
    }

    unsafe fn flush() {
        if RTT_INITIALIZED.load(Ordering::Relaxed) {
            let defmt_channel = unsafe { rtt_target::UpChannel::conjure(0).unwrap() };
            defmt_channel.flush();
        }
    }

    unsafe fn release() {
        unsafe {
            (&mut *addr_of_mut!(ENCODER)).end_frame(|bytes| write(bytes));
            // Always commit after releasing to ensure data is available to consumer
            // This ensures even small messages get committed to the buffer
            commit_write_grant();

            // Increment frame counter for debugging
            FRAME_COUNT.fetch_add(1, Ordering::Relaxed);

            critical_section::release(RESTORE_STATE);
        }
    }

    unsafe fn write(bytes: &[u8]) {
        unsafe {
            (&mut *addr_of_mut!(ENCODER)).write(bytes, |bytes| write(bytes));
        }
    }
}

/// Safety: Must be called in a critical section
unsafe fn write(bytes: &[u8]) {
    // Increment counter to track how many writes go through our custom logger
    WRITE_COUNT.fetch_add(1, Ordering::Relaxed);

    if RTT_INITIALIZED
        .compare_exchange(false, true, Ordering::Relaxed, Ordering::Relaxed)
        .is_ok()
    {
        rtt_target::rtt_init! {
            up: {
                0: { // channel number
                    size: RTT_BUFFER_SIZE, // buffer size in bytes
                    name: "DEFMT\0" // name (optional, default: no name)
                }
            }
        };
    }

    // Write to RTT first (for debugging/development)
    let mut defmt_channel = unsafe { rtt_target::UpChannel::conjure(0).unwrap() };
    let mut rtt_bytes = bytes;
    while !rtt_bytes.is_empty() {
        let written = defmt_channel.write(rtt_bytes);
        rtt_bytes = &rtt_bytes[written..];
    }

    // Then write to our circular buffer for transport
    let mut internal_bytes = bytes;
    while !internal_bytes.is_empty() {
        match unsafe { get_write_grant() } {
            Some((wg, written)) => {
                let min_len = internal_bytes.len().min(wg.len() - *written);
                wg[*written..][..min_len].copy_from_slice(&internal_bytes[..min_len]);

                *written += min_len;
                internal_bytes = &internal_bytes[min_len..];

                // If buffer is full, commit and get a new grant for remaining data
                if *written == wg.len() {
                    // Explicitly drop the references to the grant before committing
                    unsafe { commit_write_grant() };
                }
            }
            None => {
                // Buffer is full - force commit any existing grant and try to get a new one
                // This implements overwrite behavior for the circular buffer
                unsafe { commit_write_grant() };

                // Try to get a new grant after committing
                match unsafe { get_write_grant() } {
                    Some((wg, written)) => {
                        let min_len = internal_bytes.len().min(wg.len() - *written);
                        wg[*written..][..min_len].copy_from_slice(&internal_bytes[..min_len]);
                        *written += min_len;
                        internal_bytes = &internal_bytes[min_len..];
                    }
                    None => {
                        // Still can't get a grant - this shouldn't happen with BBQueue
                        // but break to avoid infinite loop
                        break;
                    }
                }
            }
        }
    }
}

/// Implementation function for the debug bytes send task with a specific transport
pub async fn defmt_bytes_send_task_impl<T: DebugTransport>(mut transport: T) {
    let framed_consumer = DEFMT_BUFFER.framed_consumer();

    defmt::debug!("Starting defmt bytes send task");

    loop {
        defmt::debug!("Waiting for defmt frame");
        let frame = framed_consumer.wait_read().await;

        // Periodically log how many writes have gone through our custom logger
        let write_count = WRITE_COUNT.load(Ordering::Relaxed);
        let frame_count = FRAME_COUNT.load(Ordering::Relaxed);
        defmt::debug!(
            "Custom logger write count: {}, frame count: {}, frame size: {}",
            write_count,
            frame_count,
            frame.len()
        );

        // Send the frame data via the configured transport
        defmt::debug!("About to send frame via OOB transport");

        if let Err(err) = transport.send(frame.deref()).await {
            // Log transport errors but continue processing
            defmt::warn!("Transport error: {:?}", defmt::Debug2Format(&err));
        } else {
            defmt::debug!("Frame sent successfully via OOB transport");
        }

        // Always release the frame to continue processing
        frame.release();

        defmt::debug!("Frame processed, continuing loop");
    }
}

/// Default embassy task with automatic transport selection based on enabled features
/// Transport priority: USB > eSPI > UART > NoOp
#[embassy_executor::task]
pub async fn defmt_bytes_send_task() {
    let transport = transport::create_default_transport();
    defmt_bytes_send_task_impl(transport).await;
}

/// Get a consumer for the debug buffer to implement notification/pull architecture
/// This allows external code to wait for notifications and pull data when available
pub fn get_buffer_consumer() -> bbq2::prod_cons::framed::FramedConsumer<&'static Queue> {
    DEFMT_BUFFER.framed_consumer()
}

/// Get a handle to the data-available signal. The eSPI service (or any other
/// consumer) can `wait().await` on this to be notified that at least one frame
/// worth of data has been committed to the internal circular buffer.
pub fn debug_data_available_signal() -> &'static Signal<CriticalSectionRawMutex, ()> {
    &DATA_AVAILABLE_SIGNAL
}
