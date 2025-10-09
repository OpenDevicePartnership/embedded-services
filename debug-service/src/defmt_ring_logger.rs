use bbq2::{
    prod_cons::framed::{FramedGrantW, FramedProducer},
    queue::BBQueue,
    traits::{coordination::cas::AtomicCoord, notifier::maitake::MaiNotSpsc, storage::Inline},
};
use core::borrow::Borrow;
use core::{
    borrow::BorrowMut,
    ops::DerefMut,
    sync::atomic::{AtomicBool, Ordering},
};
use embedded_services::ec_type::message::{StdHostPayload, StdHostRequest};

use crate::{frame_available, shared_buffer};

static RTT_INITIALIZED: AtomicBool = AtomicBool::new(false);
static mut ENCODER: defmt::Encoder = defmt::Encoder::new();
static mut RESTORE_STATE: critical_section::RestoreState = critical_section::RestoreState::invalid();

type Queue = BBQueue<Inline<4096>, AtomicCoord, MaiNotSpsc>;

static DEFMT_BUFFER: Queue = Queue::new();
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

#[embassy_executor::task]
pub async fn defmt_to_host_task() {
    embedded_services::info!("defmt to host task start");
    use crate::debug_service::{host_endpoint_id, response_notify_signal};
    use embedded_services::comms::{self, EndpointID, Internal};
    use embedded_services::ec_type::message::HostMsg;

    let framed_consumer = DEFMT_BUFFER.framed_consumer();

    let host_ep = host_endpoint_id().await;

    // Acquire the staging buffer once; we own it for the task lifetime.
    let acpi_owned = crate::owned_buffer();

    loop {
        // Wait for a complete defmt frame to be available (do not release yet)
        let frame = framed_consumer.wait_read().await;

        // Copy frame bytes into the static ACPI buffer.
        // Producer commits frames atomically with size ≤ DEFMT_MAX_BYTES (1024),
        // so the consumer never sees a partial frame. We still clamp to the
        // destination length to be robust if the staging buffer size changes.
        let copy_len = core::cmp::min(frame.len(), acpi_owned.len());
        {
            let mut access = acpi_owned.borrow_mut();
            let buf: &mut [u8] = BorrowMut::borrow_mut(&mut access);

            buf[..copy_len].copy_from_slice(&frame[..copy_len]);
        }

        frame.release();
        embedded_services::trace!("released defmt frame (staged {} bytes)", copy_len);

        // Notify the host that data is available
        // No notification for now until that's sorted. Host will periodically poll
        // TODO: Revisit once host notifications are stabilized.
        /*let _ = comms::send(
            EndpointID::Internal(Internal::Debug),
            host_ep,
            &HostMsg::Notification(NotificationMsg { offset: 20 }),
        )
        .await;*/

        // Wait for host notification/ack via the debug service
        frame_available(true);
        let _n = response_notify_signal().wait().await;
        frame_available(false);
        embedded_services::trace!("host ack received, sending defmt response");

        // Send the staged defmt bytes frame as an ACPI-style message.
        // Scope the message so the shared borrow is dropped before we clear the buffer.
        {
            let msg = HostMsg::Response(StdHostRequest {
                command: embedded_services::ec_type::message::OdpCommand::Debug(
                    embedded_services::ec_type::protocols::debug::DebugCmd::GetMsgs,
                ),
                status: 0,
                payload: StdHostPayload::DebugGetMsgsResponse {
                    debug_buf: {
                        let access = shared_buffer().borrow();
                        let slice: &[u8] = access.borrow();
                        slice.try_into().unwrap()
                    },
                },
            });
            let _ = comms::send(EndpointID::Internal(Internal::Debug), host_ep, &msg).await;
            embedded_services::trace!("sent {} defmt bytes to host", copy_len);
        }

        // Clear the staged portion of the buffer
        {
            let mut access = acpi_owned.borrow_mut();
            let buf: &mut [u8] = BorrowMut::borrow_mut(&mut access);
            buf[..copy_len].fill(0);
        }
    }
}

#[embassy_executor::task]
pub async fn no_avail_to_host_task() {
    embedded_services::define_static_buffer!(no_avail_acpi_buf, u8, [0u8; 12]);

    embedded_services::info!("no avail to host task start");
    use crate::debug_service::{host_endpoint_id, no_avail_notify_signal};
    use embedded_services::comms::{self, EndpointID, Internal};
    use embedded_services::ec_type::message::HostMsg;

    let host_ep = host_endpoint_id().await;

    let acpi_owned = no_avail_acpi_buf::get_mut().expect("defmt staging buffer already initialized elsewhere");
    {
        let mut access = acpi_owned.borrow_mut();
        let buf: &mut [u8] = BorrowMut::borrow_mut(&mut access);
        // Use 0xDEADBEEF to signify no frame available
        buf[4..12].copy_from_slice(&0xDEADBEEFu64.to_be_bytes());
    }

    let msg = HostMsg::Response(StdHostRequest {
        command: embedded_services::ec_type::message::OdpCommand::Debug(
            embedded_services::ec_type::protocols::debug::DebugCmd::GetMsgs,
        ),
        status: 1,
        payload: StdHostPayload::ErrorResponse {},
    });

    // Send DEADBEEF if host requests frame but non available
    loop {
        no_avail_notify_signal().wait().await;
        let _ = comms::send(EndpointID::Internal(Internal::Debug), host_ep, &msg).await;
    }
}
