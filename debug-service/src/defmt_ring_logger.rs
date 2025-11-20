use bbq2::{
    prod_cons::framed::{FramedGrantW, FramedProducer},
    queue::BBQueue,
    traits::{coordination::cas::AtomicCoord, notifier::maitake::MaiNotSpsc, storage::Inline},
};
use core::{
    ops::DerefMut,
    sync::atomic::{AtomicBool, Ordering},
};

static RTT_INITIALIZED: AtomicBool = AtomicBool::new(false);
static mut ENCODER: defmt::Encoder = defmt::Encoder::new();
static mut RESTORE_STATE: critical_section::RestoreState = critical_section::RestoreState::invalid();

type Queue = BBQueue<Inline<4096>, AtomicCoord, MaiNotSpsc>;

pub(crate) static DEFMT_BUFFER: Queue = Queue::new();
static mut WRITE_GRANT: Option<FramedGrantW<&'static Queue>> = None;
static mut WRITTEN: usize = 0;

/// Indicates whether the start frame should be written on the first [`defmt::Logger::write`].
///
/// A start frame is typically written in [`defmt::Logger::acquire`].
/// However, we may not want to send the frame if that frame's log level is disabled, which can only be
/// detected in the first [`defmt::Logger::write`].
/// If we always wrote the start frame in the first [`defmt::Logger::acquire`], we'll sometimes have an empty frame.
/// To avoid this, we defer writing the start frame to the first [`defmt::Logger::write`] then update this
/// variable to indicate that the start frame has been written.
///
/// # Safety
/// This variable should be read or written to when the critical section is acquired in [`RESTORE_STATE`].
static mut START_FRAME: bool = true;

/// Safety:
/// Only one producer reference may exist at one time
#[allow(clippy::deref_addrof)]
unsafe fn get_producer() -> &'static mut FramedProducer<&'static Queue> {
    static mut PRODUCER: Option<FramedProducer<&'static Queue>> = None;

    let producer = unsafe { &mut *(&raw mut PRODUCER) };

    match producer {
        Some(p) => p,
        None => producer.insert(DEFMT_BUFFER.framed_producer()),
    }
}

/// Safety:
/// Only one grant reference may exist at one time
#[allow(clippy::deref_addrof)]
unsafe fn get_write_grant() -> Option<(&'static mut [u8], &'static mut usize)> {
    let write_grant = unsafe { &mut *&raw mut WRITE_GRANT };

    let write_grant = match write_grant {
        Some(wg) => wg,
        wg @ None => wg.insert(unsafe { get_producer() }.grant(crate::DEFMT_MAX_BYTES).ok()?),
    };

    Some((write_grant.deref_mut(), unsafe { &mut *&raw mut WRITTEN }))
}

unsafe fn commit_write_grant() {
    #[allow(clippy::deref_addrof)]
    if let Some(wg) = unsafe { &mut *&raw mut WRITE_GRANT }.take() {
        wg.commit(unsafe { WRITTEN } as u16)
    }

    unsafe {
        WRITTEN = 0;
    }
}

#[defmt::global_logger]
struct DefmtLogger;
#[allow(clippy::deref_addrof)]
unsafe impl defmt::Logger for DefmtLogger {
    fn acquire() {
        unsafe {
            RESTORE_STATE = critical_section::acquire();
            // Reset print state
            START_FRAME = true;
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
            (&mut *&raw mut ENCODER).end_frame(|bytes| write(bytes));
            commit_write_grant();
            critical_section::release(RESTORE_STATE);
        }
    }

    unsafe fn write(bytes: &[u8]) {
        unsafe {
            if START_FRAME {
                // Start a new frame on the first write of this log event
                (&mut *&raw mut ENCODER).start_frame(|bytes| write(bytes));
                START_FRAME = false;
            }
            (&mut *&raw mut ENCODER).write(bytes, |bytes| write(bytes));
        }
    }
}

/// Safety: Must be called in a critical section
unsafe fn write(bytes: &[u8]) {
    if RTT_INITIALIZED
        .compare_exchange(false, true, Ordering::Relaxed, Ordering::Relaxed)
        .is_ok()
    {
        rtt_target::rtt_init! {
            up: {
                0: { // channel number
                    size: 4096, // buffer size in bytes
                    name: "defmt" // name (optional, default: no name)
                }
            }
        };
    }

    let mut internal_bytes = bytes;
    while !internal_bytes.is_empty() {
        match unsafe { get_write_grant() } {
            Some((wg, written)) => {
                let (should_commit, min_len) = {
                    let wg_len = wg.len();
                    let min_len = internal_bytes.len().min(wg_len - *written);
                    wg[*written..][..min_len].copy_from_slice(&internal_bytes[..min_len]);
                    *written += min_len;
                    (*written == wg_len, min_len)
                };

                if should_commit {
                    unsafe { commit_write_grant() };
                }

                internal_bytes = &internal_bytes[min_len..];
            }
            None => {
                // We're full. Not much we can do
                break;
            }
        }
    }

    let mut defmt_channel = unsafe { rtt_target::UpChannel::conjure(0).unwrap() };

    let mut rtt_bytes = bytes;
    while !rtt_bytes.is_empty() {
        let written = defmt_channel.write(rtt_bytes);
        if written == 0 {
            // RTT buffer is full (no host connected), give up on remaining bytes
            break;
        }
        rtt_bytes = &rtt_bytes[written..];
    }
}
