//! uart-service
//!
//! UART transport for MCTP packets, generic over [`mctp_rs::MctpMedium`].
//! Use [`DefaultService`] for the SmbusEspi-medium baseline; use
//! [`Service::new`] directly with another medium (e.g. DSP0253 serial)
//! for non-SmbusEspi callers.
//!
//! Revisit: Will also need to consider how to handle notifications (likely need to have user
//! provide GPIO pin we can use).
#![no_std]

pub mod task;

use embassy_sync::channel::Channel;
use embedded_io_async::Read as UartRead;
use embedded_io_async::Write as UartWrite;
use embedded_services::GlobalRawMutex;
use embedded_services::relay::mctp::{RelayHandler, RelayHeader, RelayResponse};
use embedded_services::trace;
use mctp_rs::MctpMedium;
use mctp_rs::smbus_espi::{SmbusEspiMedium, SmbusEspiReplyContext};

// Should be as large as the largest possible MCTP packet and its metadata.
const BUF_SIZE: usize = 256;
const HOST_TX_QUEUE_SIZE: usize = 5;

#[derive(Clone)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub(crate) struct HostResultMessage<R: RelayHandler> {
    pub handler_service_id: R::ServiceIdType,
    pub message: R::ResultEnumType,
}

#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Error<M: MctpMedium> {
    /// Comms error.
    Comms,
    /// UART error.
    Uart,
    /// MCTP serialization error.
    Mctp(mctp_rs::MctpPacketError<M>),
    /// Other serialization error.
    Serialize(&'static str),
    /// Buffer error.
    Buffer(embedded_services::buffer::Error),
}

/// UART-driven MCTP relay service, generic over the medium `M`.
///
/// # `M: Copy` bound
///
/// The `Copy` bound on `M` is required because [`MctpPacketContext`]
/// takes the medium by value, and `Service` needs to construct a fresh
/// `MctpPacketContext` on each request and each response (the
/// `MctpPacketContext` borrows the assembly buffer for the duration of
/// one packet, so it must be re-created per round-trip). Storing `M` by
/// value and copying it into each new context is the simplest shape;
/// it's free for both shipped media (`SmbusEspiMedium`, `MctpSerialMedium`
/// are zero-sized types). A future medium with internal state that
/// cannot be `Copy` would need either an `&'_ M`-based redesign of
/// `MctpPacketContext` or an interior-mutability wrapper.
///
/// [`MctpPacketContext`]: mctp_rs::MctpPacketContext
pub struct Service<R: RelayHandler, M: MctpMedium + Copy> {
    host_tx_queue: Channel<GlobalRawMutex, HostResultMessage<R>, HOST_TX_QUEUE_SIZE>,
    relay_handler: R,
    medium: M,
    reply_context: mctp_rs::MctpReplyContext<M>,
}

impl<R: RelayHandler, M: MctpMedium + Copy> Service<R, M> {
    pub fn new(relay_handler: R, medium: M, reply_context: mctp_rs::MctpReplyContext<M>) -> Result<Self, Error<M>> {
        Ok(Self {
            host_tx_queue: Channel::new(),
            relay_handler,
            medium,
            reply_context,
        })
    }

    async fn process_response<T: UartWrite>(
        &self,
        uart: &mut T,
        response: HostResultMessage<R>,
    ) -> Result<(), Error<M>> {
        let mut assembly_buf = [0u8; BUF_SIZE];
        let mut mctp_ctx = mctp_rs::MctpPacketContext::<M>::new(self.medium, &mut assembly_buf);

        // Start from the stored reply_context, override the per-response
        // destination_endpoint_id from the responding service.
        let mut reply_context = self.reply_context;
        reply_context.destination_endpoint_id = mctp_rs::EndpointId::Id(response.handler_service_id.into());

        let header = response.message.create_header(&response.handler_service_id);
        let mut packet_state = mctp_ctx
            .serialize_packet(reply_context, (header, response.message))
            .map_err(Error::Mctp)?;

        while let Some(packet_result) = packet_state.next() {
            let packet = packet_result.map_err(Error::Mctp)?;

            // Then actually send the response packet (medium framing already applied)
            uart.write_all(packet).await.map_err(|_| Error::Uart)?;
        }

        Ok(())
    }

    async fn wait_for_request<T: UartRead>(&self, uart: &mut T) -> Result<(), Error<M>> {
        // Incremental read loop: read bytes, ask the medium whether the
        // assembled prefix is a complete frame, repeat until it is.
        let mut buf = [0u8; BUF_SIZE];
        let mut filled = 0usize;
        let packet_len = loop {
            let dst = buf.get_mut(filled..).ok_or(Error::Serialize("buffer overrun"))?;
            if dst.is_empty() {
                return Err(Error::Serialize("frame exceeds BUF_SIZE"));
            }
            let n = uart.read(dst).await.map_err(|_| Error::Uart)?;
            if n == 0 {
                return Err(Error::Comms);
            }
            filled += n;
            match self
                .medium
                .frame_complete(buf.get(..filled).ok_or(Error::Serialize("buffer overrun"))?)
                .map_err(Error::Mctp)?
            {
                Some(len) => break len,
                None => continue,
            }
        };

        let mut assembly_buf = [0u8; BUF_SIZE];
        let mut mctp_ctx = mctp_rs::MctpPacketContext::<M>::new(self.medium, &mut assembly_buf);

        let message = mctp_ctx
            .deserialize_packet(
                buf.get(..packet_len)
                    .ok_or(Error::Serialize("frame exceeds BUF_SIZE"))?,
            )
            .map_err(Error::Mctp)?
            .ok_or(Error::Serialize("Partial message not supported"))?;

        let (header, body) = message.parse_as::<R::RequestEnumType>().map_err(Error::Mctp)?;
        trace!("Received host request");

        let response = self.relay_handler.process_request(body).await;
        self.host_tx_queue
            .try_send(HostResultMessage {
                handler_service_id: header.get_service_id(),
                message: response,
            })
            .map_err(|_| Error::Comms)?;

        Ok(())
    }

    async fn wait_for_response(&self) -> HostResultMessage<R> {
        self.host_tx_queue.receive().await
    }
}

/// Backwards-compatible alias for SmbusEspi-medium services.
pub type DefaultService<R> = Service<R, SmbusEspiMedium>;

impl<R: RelayHandler> DefaultService<R> {
    /// Constructor for SmbusEspi-medium services. Hardcodes the
    /// reply-context addressing used by the existing SmbusEspi
    /// consumers (destination_slave_address: 1, source_slave_address: 0).
    /// The `destination_endpoint_id` is overridden per-response inside
    /// `process_response`, so the value passed here is a placeholder.
    pub fn default_smbusespi(relay_handler: R) -> Result<Self, Error<SmbusEspiMedium>> {
        Self::new(
            relay_handler,
            SmbusEspiMedium,
            mctp_rs::MctpReplyContext {
                source_endpoint_id: mctp_rs::EndpointId::Id(0x80),
                destination_endpoint_id: mctp_rs::EndpointId::Id(0),
                packet_sequence_number: mctp_rs::MctpSequenceNumber::new(0),
                message_tag: mctp_rs::MctpMessageTag::try_from(3).map_err(Error::Serialize)?,
                medium_context: SmbusEspiReplyContext {
                    destination_slave_address: 1,
                    source_slave_address: 0,
                },
            },
        )
    }
}

/// Type alias for `MctpSerialMedium` services (DSP0253-style framed
/// serial, no per-medium addressing). Used by the QEMU EC ↔ SP relay
/// path where the secure PL011 is bridged via a host PTY.
pub type MctpSerialService<R> = Service<R, mctp_rs::MctpSerialMedium>;

impl<R: RelayHandler> MctpSerialService<R> {
    /// Constructor for `MctpSerialMedium` services. Hardcodes the
    /// EC ↔ SP reply context (`source = EC_EID`, `message_tag = 0`,
    /// `medium_context = ()`). The `destination_endpoint_id` is
    /// overridden per-response inside `process_response`, so the
    /// `SP_EID` passed here is a placeholder.
    pub fn default_mctp_serial(relay_handler: R) -> Result<Self, Error<mctp_rs::MctpSerialMedium>> {
        Self::new(
            relay_handler,
            mctp_rs::MctpSerialMedium,
            mctp_rs::MctpReplyContext {
                source_endpoint_id: mctp_rs::EC_EID,
                destination_endpoint_id: mctp_rs::SP_EID,
                packet_sequence_number: mctp_rs::MctpSequenceNumber::new(0),
                message_tag: mctp_rs::MctpMessageTag::try_from(0).map_err(Error::Serialize)?,
                medium_context: (),
            },
        )
    }
}
