use core::slice;

use crate::mctp::{HostRequest, HostResult, OdpHeader, OdpMessageType, OdpService};
use core::borrow::BorrowMut;
use embassy_imxrt::espi;
use embassy_sync::channel::Channel;
use embassy_sync::once_lock::OnceLock;
use embedded_services::buffer::OwnedRef;
use embedded_services::comms::{self, EndpointID, External};
use embedded_services::{GlobalRawMutex, debug, error, info, trace};
use mctp_rs::smbus_espi::SmbusEspiMedium;
use mctp_rs::smbus_espi::SmbusEspiReplyContext;

const HOST_TX_QUEUE_SIZE: usize = 5;

// OOB port number for NXP IMXRT
// REVISIT: When adding support for other platforms, refactor this as they don't have a notion of port IDs
const OOB_PORT_ID: usize = 1;

// Should be as large as the largest possible MCTP packet and its metadata.
const ASSEMBLY_BUF_SIZE: usize = 256;

embedded_services::define_static_buffer!(assembly_buf, u8, [0u8; ASSEMBLY_BUF_SIZE]);

#[derive(Clone)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub(crate) struct HostResultMessage {
    pub source_endpoint: EndpointID,
    pub message: HostResult,
}

#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Error {
    Serialize,
    Buffer(embedded_services::buffer::Error),
}

pub struct Service<'a> {
    endpoint: comms::Endpoint,
    host_tx_queue: Channel<GlobalRawMutex, HostResultMessage, HOST_TX_QUEUE_SIZE>,
    assembly_buf_owned_ref: OwnedRef<'a, u8>,
}

impl Service<'_> {
    pub fn new() -> Self {
        Service {
            endpoint: comms::Endpoint::uninit(EndpointID::External(External::Host)),
            host_tx_queue: Channel::new(),
            assembly_buf_owned_ref: assembly_buf::get_mut().unwrap(),
        }
    }

    pub(crate) async fn wait_for_response(&self) -> HostResultMessage {
        self.host_tx_queue.receive().await
    }

    // TODO The notification system was not actually used, so this is currently dead code.
    //      We need to implement some interface for triggering notifications from other subsystems, and it may do something like this:
    //
    // async fn process_notification_to_host(&self, espi: &mut espi::Espi<'_>, notification: &NotificationMsg) {
    //     espi.irq_push(notification.offset).await;
    //     info!("espi: Notification id {} sent to Host!", notification.offset);
    // }

    async fn serialize_packet_from_subsystem(
        &self,
        espi: &mut espi::Espi<'static>,
        result: &HostResultMessage,
    ) -> Result<(), Error> {
        let mut assembly_buf_access = self.assembly_buf_owned_ref.borrow_mut().map_err(Error::Buffer)?;
        let pkt_ctx_buf = assembly_buf_access.borrow_mut();
        let mut mctp_ctx = mctp_rs::MctpPacketContext::new(mctp_rs::smbus_espi::SmbusEspiMedium, pkt_ctx_buf);

        let source_service: OdpService = OdpService::try_from(result.source_endpoint).map_err(|_| Error::Serialize)?;

        let reply_context: mctp_rs::MctpReplyContext<SmbusEspiMedium> = mctp_rs::MctpReplyContext {
            source_endpoint_id: mctp_rs::EndpointId::Id(0x80),
            destination_endpoint_id: mctp_rs::EndpointId::Id(source_service.into()), // TODO We're currently using this incorrectly - it should be the bus address of the host. Revisit once we have assigned a bus address to the host.
            packet_sequence_number: mctp_rs::MctpSequenceNumber::new(0),
            message_tag: mctp_rs::MctpMessageTag::try_from(3).map_err(|e| {
                error!("serialize_packet_from_subsystem: {:?}", e);
                Error::Serialize
            })?,
            medium_context: SmbusEspiReplyContext {
                destination_slave_address: 1,
                source_slave_address: 0,
            }, // Medium-specific context
        };

        let header = OdpHeader {
            message_type: OdpMessageType::Result {
                is_error: !result.message.is_ok(),
            },
            is_datagram: false,
            service: source_service,
            message_id: result.message.discriminant(),
        };

        let mut packet_state = mctp_ctx
            .serialize_packet(reply_context, (header, result.message.clone()))
            .map_err(|e| {
                error!("serialize_packet_from_subsystem: {:?}", e);
                Error::Serialize
            })?;
        // Send each packet
        while let Some(packet_result) = packet_state.next() {
            let packet = packet_result.map_err(|e| {
                error!("serialize_packet_from_subsystem: {:?}", e);
                Error::Serialize
            })?;
            // Last byte is PEC, ignore for now
            let packet = &packet[..packet.len() - 1];
            trace!("Sending MCTP response: {:?}", packet);

            self.write_to_hw(espi, packet).map_err(|e| {
                error!("serialize_packet_from_subsystem: {:?}", e);
                Error::Serialize
            })?;

            // Immediately service the packet with the ESPI HAL
            let event = espi.wait_for_event().await;
            process_controller_event(espi, self, event).await?;
        }
        Ok(())
    }

    fn write_to_hw(&self, espi: &mut espi::Espi<'static>, packet: &[u8]) -> Result<(), embassy_imxrt::espi::Error> {
        // Send packet via your transport medium
        // SAFETY: Safe as the access to espi is protected by a mut reference.
        let dest_slice = unsafe { espi.oob_get_write_buffer(OOB_PORT_ID)? };
        dest_slice[..packet.len()].copy_from_slice(&packet[..packet.len()]);

        // Write response over OOB
        espi.oob_write_data(OOB_PORT_ID, packet.len() as u8)
    }

    async fn send_mctp_error_response(&self, endpoint: EndpointID, espi: &mut espi::Espi<'static>) {
        // TODO we may want to add more detail in future, but that will require more integration with the debug service
        let error_msg = HostResultMessage {
            source_endpoint: endpoint,
            message: HostResult::Debug(Err(debug_service_messages::DebugError::UnspecifiedFailure)),
        };
        self.serialize_packet_from_subsystem(espi, &error_msg)
            .await
            .unwrap_or_else(|_| {
                error!("Critical error reporting MCTP protocol error to host!");
            });
    }

    pub(crate) async fn process_response_to_host(&self, espi: &mut espi::Espi<'static>, response: HostResultMessage) {
        match self.serialize_packet_from_subsystem(espi, &response).await {
            Err(e) => {
                error!("Packet serialize error {:?}", e);

                self.send_mctp_error_response(response.source_endpoint, espi).await;
            }
            Ok(()) => {
                trace!("Full packet successfully sent to host!")
            }
        }
    }

    pub(crate) fn endpoint(&self) -> &comms::Endpoint {
        &self.endpoint
    }

    fn queue_response_to_host(
        &self,
        source_endpoint: EndpointID,
        message: HostResult,
    ) -> Result<(), comms::MailboxDelegateError> {
        debug!("Espi service: recvd response");
        self.host_tx_queue
            .try_send(HostResultMessage {
                source_endpoint,
                message,
            })
            .map_err(|_| comms::MailboxDelegateError::BufferFull)?;

        Ok(())
    }
}

impl comms::MailboxDelegate for Service<'_> {
    fn receive(&self, message: &comms::Message) -> Result<(), comms::MailboxDelegateError> {
        crate::mctp::send_to_comms(message, |source_endpoint, message| {
            self.queue_response_to_host(source_endpoint, message)
        })
    }
}

pub(crate) static ESPI_SERVICE: OnceLock<Service> = OnceLock::new();

pub(crate) async fn process_controller_event(
    espi: &mut espi::Espi<'static>,
    espi_service: &Service<'_>,
    event: Result<embassy_imxrt::espi::Event, embassy_imxrt::espi::Error>,
) -> Result<(), Error> {
    match event {
        Ok(espi::Event::PeripheralEvent(port_event)) => {
            info!(
                "eSPI PeripheralEvent Port: {}, direction: {}, address: {}, offset: {}, length: {}",
                port_event.port, port_event.direction, port_event.offset, port_event.base_addr, port_event.length,
            );

            // We're not handling these - communication is all through OOB

            espi.complete_port(port_event.port);
        }
        Ok(espi::Event::OOBEvent(port_event)) => {
            info!(
                "eSPI OOBEvent Port: {}, direction: {}, address: {}, offset: {}, length: {}",
                port_event.port, port_event.direction, port_event.offset, port_event.base_addr, port_event.length,
            );

            if port_event.direction {
                let src_slice = unsafe { slice::from_raw_parts(port_event.base_addr as *const u8, port_event.length) };

                // TODO: This is a workaround because mctp_rs expects a PEC byte, so we hardcode a 0 at the end.
                // We should add functionality to mctp_rs to disable PEC.
                let mut with_pec = [0u8; 100];
                with_pec[..src_slice.len()].copy_from_slice(src_slice);
                with_pec[src_slice.len()] = 0;
                let with_pec = &with_pec[..=src_slice.len()];

                #[cfg(feature = "defmt")] // Required because without defmt, there is no implementation of UpperHex for [u8]
                debug!("OOB message: {:02X}", &src_slice[0..]);

                let mut assembly_access = espi_service
                    .assembly_buf_owned_ref
                    .borrow_mut()
                    .map_err(Error::Buffer)?;
                let mut mctp_ctx =
                    mctp_rs::MctpPacketContext::<SmbusEspiMedium>::new(SmbusEspiMedium, assembly_access.borrow_mut());

                match mctp_ctx.deserialize_packet(with_pec) {
                    Ok(Some(message)) => {
                        trace!("MCTP packet successfully deserialized");

                        match message.parse_as::<HostRequest>() {
                            Ok((header, body)) => {
                                let target_endpoint = header.service.get_endpoint_id();
                                trace!(
                                    "Host Request: Service {:?}, Command {:?}",
                                    target_endpoint, header.message_id,
                                );

                                drop(assembly_access);

                                espi.complete_port(port_event.port);

                                body.send_to_endpoint(&espi_service.endpoint, target_endpoint)
                                    .await
                                    .expect("result error type is infallible");
                                info!("MCTP packet forwarded to service: {:?}", target_endpoint);
                            }
                            Err(e) => {
                                error!("MCTP ODP type malformed: {:?}", e);

                                espi.complete_port(port_event.port);

                                return Err(Error::Serialize);
                            }
                        }
                    }
                    Ok(None) => {
                        // Partial message, waiting for more packets
                        error!("Partial msg, should not happen");
                        espi.complete_port(OOB_PORT_ID);

                        return Err(Error::Serialize);
                    }
                    Err(_e) => {
                        // Handle protocol or medium error
                        error!("MCTP packet malformed");

                        error!("error code: {:?}", _e);
                        espi.complete_port(OOB_PORT_ID);

                        return Err(Error::Serialize);
                    }
                }
            } else {
                espi.complete_port(port_event.port);
            }
        }
        Ok(espi::Event::Port80) => {
            info!("eSPI Port 80");
        }
        Ok(espi::Event::WireChange(_)) => {
            info!("eSPI WireChange");
        }
        Err(e) => {
            error!("eSPI Failed with error: {:?}", e);
        }
    }
    Ok(())
}
