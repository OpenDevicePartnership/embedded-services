//! uart-service
//!
//! To keep things consistent with eSPI service, this also uses the `SmbusEspiMedium` (though not
//! strictly necessary, this helps minimize code changes on the host side when swicthing between
//! eSPI or UART).
//!
//! Revisit: Will also need to consider how to handle notifications (likely need to have user
//! provide GPIO pin we can use).
#![no_std]

mod mctp;
pub mod task;

use crate::mctp::{HostRequest, HostResult, OdpHeader, OdpMessageType, OdpService};
use core::borrow::BorrowMut;
use embassy_sync::channel::Channel;
use embedded_io_async::Read as UartRead;
use embedded_io_async::Write as UartWrite;
use embedded_services::GlobalRawMutex;
use embedded_services::buffer::OwnedRef;
use embedded_services::comms::{self, Endpoint, EndpointID, External};
use embedded_services::trace;
use mctp_rs::smbus_espi::SmbusEspiMedium;
use mctp_rs::smbus_espi::SmbusEspiReplyContext;

// Should be as large as the largest possible MCTP packet and its metadata.
const BUF_SIZE: usize = 256;
const HOST_TX_QUEUE_SIZE: usize = 5;
const SMBUS_HEADER_SIZE: usize = 4;
const SMBUS_LEN_IDX: usize = 2;

embedded_services::define_static_buffer!(assembly_buf, u8, [0u8; BUF_SIZE]);

#[derive(Clone)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub(crate) struct HostResponseMessage {
    pub source_endpoint: EndpointID,
    pub message: HostResult,
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

pub struct Service<'a> {
    endpoint: Endpoint,
    host_tx_queue: Channel<GlobalRawMutex, HostResponseMessage, HOST_TX_QUEUE_SIZE>,
    assembly_buf_owned_ref: OwnedRef<'a, u8>,
}

impl Service<'_> {
    pub fn new() -> Result<Self, Error> {
        Ok(Self {
            endpoint: Endpoint::uninit(EndpointID::External(External::Host)),
            host_tx_queue: Channel::new(),
            assembly_buf_owned_ref: assembly_buf::get_mut()
                .ok_or(Error::Buffer(embedded_services::buffer::Error::InvalidRange))?,
        })
    }

    async fn process_response<T: UartWrite>(&self, uart: &mut T, response: &HostResponseMessage) -> Result<(), Error> {
        let mut assembly_buf_access = self.assembly_buf_owned_ref.borrow_mut().map_err(Error::Buffer)?;
        let pkt_ctx_buf = assembly_buf_access.borrow_mut();
        let mut mctp_ctx = mctp_rs::MctpPacketContext::new(SmbusEspiMedium, pkt_ctx_buf);

        let source_service: OdpService = OdpService::try_from(response.source_endpoint).map_err(|_| Error::Comms)?;

        let reply_context: mctp_rs::MctpReplyContext<SmbusEspiMedium> = mctp_rs::MctpReplyContext {
            source_endpoint_id: mctp_rs::EndpointId::Id(0x80),
            destination_endpoint_id: mctp_rs::EndpointId::Id(source_service.into()),
            packet_sequence_number: mctp_rs::MctpSequenceNumber::new(0),
            message_tag: mctp_rs::MctpMessageTag::try_from(3).map_err(Error::Serialize)?,
            medium_context: SmbusEspiReplyContext {
                destination_slave_address: 1,
                source_slave_address: 0,
            }, // Medium-specific context
        };

        let header = OdpHeader {
            message_type: OdpMessageType::Result {
                is_error: !response.message.is_ok(),
            },
            is_datagram: false,
            service: source_service,
            message_id: response.message.discriminant(),
        };

        let mut packet_state = mctp_ctx
            .serialize_packet(reply_context, (header, response.message.clone()))
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
        let mut assembly_access = self.assembly_buf_owned_ref.borrow_mut().map_err(Error::Buffer)?;
        let mut mctp_ctx =
            mctp_rs::MctpPacketContext::<SmbusEspiMedium>::new(SmbusEspiMedium, assembly_access.borrow_mut());

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

        let (header, host_request) = message.parse_as::<HostRequest>().map_err(Error::Mctp)?;
        let target_endpoint: EndpointID = header.service.get_endpoint_id();
        trace!(
            "Host Request: Service {:?}, Command {:?}",
            target_endpoint, header.message_id,
        );

        host_request
            .send_to_endpoint(&self.endpoint, target_endpoint)
            .await
            .map_err(|_| Error::Comms)?;

        Ok(())
    }

    async fn wait_for_response(&self) -> HostResponseMessage {
        self.host_tx_queue.receive().await
    }
}

impl comms::MailboxDelegate for Service<'_> {
    fn receive(&self, message: &comms::Message) -> Result<(), comms::MailboxDelegateError> {
        crate::mctp::send_to_comms(message, |source_endpoint, message| {
            self.host_tx_queue
                .try_send(HostResponseMessage {
                    source_endpoint,
                    message,
                })
                .map_err(|_| comms::MailboxDelegateError::BufferFull)
        })
    }
}
