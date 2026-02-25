//! uart-service
//!
//! To keep things consistent with eSPI service, this also uses the `SmbusEspiMedium` (though not
//! strictly necessary, this helps minimize code changes on the host side when swicthing between
//! eSPI or UART).
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
use mctp_rs::smbus_espi::SmbusEspiMedium;
use mctp_rs::smbus_espi::SmbusEspiReplyContext;

// Should be as large as the largest possible MCTP packet and its metadata.
const BUF_SIZE: usize = 256;
const HOST_TX_QUEUE_SIZE: usize = 5;
const SMBUS_HEADER_SIZE: usize = 4;
const SMBUS_LEN_IDX: usize = 2;

#[derive(Clone)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub(crate) struct HostResultMessage<R: RelayHandler> {
    pub handler_service_id: R::ServiceIdType,
    pub message: R::ResultEnumType,
}

#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Error {
    /// Comms error.
    Comms,
    /// UART error.
    Uart,
    /// MCTP serialization error.
    Mctp(mctp_rs::MctpPacketError<SmbusEspiMedium>),
    /// Other serialization error.
    Serialize(&'static str),
    /// Index/slice error.
    IndexSlice,
    /// Buffer error.
    Buffer(embedded_services::buffer::Error),
}

pub struct Service<R: RelayHandler> {
    host_tx_queue: Channel<GlobalRawMutex, HostResultMessage<R>, HOST_TX_QUEUE_SIZE>,
    relay_handler: R,
}

impl<R: RelayHandler> Service<R> {
    pub fn new(relay_handler: R) -> Result<Self, Error> {
        Ok(Self {
            host_tx_queue: Channel::new(),
            relay_handler,
        })
    }

    async fn process_response<T: UartWrite>(&self, uart: &mut T, response: HostResultMessage<R>) -> Result<(), Error> {
        let mut assembly_buf = [0u8; BUF_SIZE];
        let mut mctp_ctx = mctp_rs::MctpPacketContext::new(SmbusEspiMedium, &mut assembly_buf);

        let reply_context: mctp_rs::MctpReplyContext<SmbusEspiMedium> = mctp_rs::MctpReplyContext {
            source_endpoint_id: mctp_rs::EndpointId::Id(0x80),
            destination_endpoint_id: mctp_rs::EndpointId::Id(response.handler_service_id.into()),
            packet_sequence_number: mctp_rs::MctpSequenceNumber::new(0),
            message_tag: mctp_rs::MctpMessageTag::try_from(3).map_err(Error::Serialize)?,
            medium_context: SmbusEspiReplyContext {
                destination_slave_address: 1,
                source_slave_address: 0,
            }, // Medium-specific context
        };

        let header = response.message.create_header(&response.handler_service_id);
        let mut packet_state = mctp_ctx
            .serialize_packet(reply_context, (header, response.message))
            .map_err(Error::Mctp)?;

        while let Some(packet_result) = packet_state.next() {
            let packet = packet_result.map_err(Error::Mctp)?;
            // Last byte is PEC, ignore for now
            let packet = packet.get(..packet.len() - 1).ok_or(Error::IndexSlice)?;

            // Then actually send the response packet (which includes 4-byte SMBUS header containing payload size)
            uart.write_all(packet).await.map_err(|_| Error::Uart)?;
        }

        Ok(())
    }

    async fn wait_for_request<T: UartRead>(&self, uart: &mut T) -> Result<(), Error> {
        let mut assembly_buf = [0u8; BUF_SIZE];
        let mut mctp_ctx = mctp_rs::MctpPacketContext::<SmbusEspiMedium>::new(SmbusEspiMedium, &mut assembly_buf);

        // First wait for SMBUS header, which tells us how big the incoming packet is
        let mut buf = [0; BUF_SIZE];
        uart.read_exact(buf.get_mut(..SMBUS_HEADER_SIZE).ok_or(Error::IndexSlice)?)
            .await
            .map_err(|_| Error::Uart)?;

        // Then wait until we've received the full payload
        let len = *buf.get(SMBUS_LEN_IDX).ok_or(Error::IndexSlice)? as usize;
        uart.read_exact(
            buf.get_mut(SMBUS_HEADER_SIZE..SMBUS_HEADER_SIZE + len)
                .ok_or(Error::IndexSlice)?,
        )
        .await
        .map_err(|_| Error::Uart)?;

        let message = mctp_ctx
            .deserialize_packet(&buf)
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
