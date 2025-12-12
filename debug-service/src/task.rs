use core::borrow::{Borrow, BorrowMut};

use debug_service_messages::{DebugError, DebugResponse};
use embedded_services::comms;

use crate::{debug_service_entry, defmt_ring_logger::DEFMT_BUFFER, frame_available, shared_buffer};

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Buffer(embedded_services::buffer::Error),
}

pub async fn debug_service(endpoint: comms::Endpoint) {
    debug_service_entry(endpoint).await;
}

pub async fn defmt_to_host_task() -> Result<embedded_services::Never, Error> {
    embedded_services::info!("defmt to host task start");
    use crate::debug_service::{host_endpoint_id, response_notify_signal};
    use embedded_services::comms::{self, EndpointID, Internal};

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
            let mut access = acpi_owned.borrow_mut().map_err(Error::Buffer)?;
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
            let msg = DebugResponse::DebugGetMsgsResponse {
                debug_buf: {
                    let access = shared_buffer().borrow().map_err(Error::Buffer)?;
                    let slice: &[u8] = access.borrow();
                    slice.try_into().unwrap()
                },
            };
            let _ = comms::send(EndpointID::Internal(Internal::Debug), host_ep, &msg).await;
            embedded_services::trace!("sent {} defmt bytes to host", copy_len);
        }

        // Clear the staged portion of the buffer
        {
            let mut access = acpi_owned.borrow_mut().map_err(Error::Buffer)?;
            let buf: &mut [u8] = BorrowMut::borrow_mut(&mut access);
            buf[..copy_len].fill(0);
        }
    }
}

pub async fn no_avail_to_host_task() -> Result<embedded_services::Never, Error> {
    embedded_services::define_static_buffer!(no_avail_acpi_buf, u8, [0u8; 12]);

    embedded_services::info!("no avail to host task start");
    use crate::debug_service::{host_endpoint_id, no_avail_notify_signal};
    use embedded_services::comms::{self, EndpointID, Internal};

    let host_ep = host_endpoint_id().await;

    let acpi_owned = no_avail_acpi_buf::get_mut().expect("defmt staging buffer already initialized elsewhere");
    {
        let mut access = acpi_owned.borrow_mut().map_err(Error::Buffer)?;
        let buf: &mut [u8] = BorrowMut::borrow_mut(&mut access);
        // Use 0xDEADBEEF to signify no frame available
        buf[4..12].copy_from_slice(&0xDEADBEEFu64.to_be_bytes());
    }

    let msg: Result<DebugResponse, DebugError> = Err(DebugError::UnspecifiedFailure);

    // Send DEADBEEF if host requests frame but non available
    loop {
        no_avail_notify_signal().wait().await;
        let _ = comms::send(EndpointID::Internal(Internal::Debug), host_ep, &msg).await;
    }
}
