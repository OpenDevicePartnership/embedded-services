use core::slice;

use crate::mctp::{HostRequest, HostResult, OdpHeader, OdpMessageType, OdpService};
use embassy_futures::select::select;
use embassy_imxrt::espi;
use embassy_sync::channel::Channel;
use embassy_sync::mutex::Mutex;
use embassy_sync::once_lock::OnceLock;
use embedded_services::comms as DEPRECATED_comms;
use embedded_services::{GlobalRawMutex, debug, error, info, trace};
use mctp_rs::smbus_espi::SmbusEspiMedium;
use mctp_rs::smbus_espi::SmbusEspiReplyContext;

const HOST_TX_QUEUE_SIZE: usize = 5;

// OOB port number for NXP IMXRT
// REVISIT: When adding support for other platforms, refactor this as they don't have a notion of port IDs
const OOB_PORT_ID: usize = 1;

// Should be as large as the largest possible MCTP packet and its metadata.
const ASSEMBLY_BUF_SIZE: usize = 256;

#[derive(Clone)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
struct LegacyHostResultMessage {
    pub source_endpoint: DEPRECATED_comms::EndpointID,
    pub message: HostResult,
}

#[derive(Clone)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
struct HostResultMessage<RelayHandler: embedded_services::relay::mctp::RelayHandler> {
    pub handler_service_id: RelayHandler::ServiceIdType,
    pub message: RelayHandler::ResultEnumType,
}

// TODO While we're migrating from the comms service to direct async calls, we need to support both the old and new message types,
//     so we use this enum to route them to the correct processing function. Once migration is complete, we should remove this and
//     just use RelayHandler::ResultEnumType everywhere.
//
enum HostResultMessageMigrationEnum<RelayHandler: embedded_services::relay::mctp::RelayHandler> {
    Legacy(LegacyHostResultMessage),
    New(HostResultMessage<RelayHandler>),
}

#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Error {
    Serialize,
    Buffer(embedded_services::buffer::Error),
}

pub struct Service<RelayHandler: embedded_services::relay::mctp::RelayHandler> {
    endpoint: DEPRECATED_comms::Endpoint,
    espi: Mutex<GlobalRawMutex, espi::Espi<'static>>,
    host_tx_queue: Channel<GlobalRawMutex, HostResultMessageMigrationEnum<RelayHandler>, HOST_TX_QUEUE_SIZE>,
    relay_handler: RelayHandler,
}

// TODO we're currently transitioning from the comms service to direct async calls to support better testing, reduced code size, and better performance.
//      As part of this transition, each service that interacts with the eSPI service needs to migrate to expose a direct async call API and implement
//      some additional traits to be able to interface with relay services in the new way.
//
//      Until all services have been migrated, we need to support both the old and new methods for interfacing with services.  Once migration is complete,
//      we can remove all the legacy code that supports the old comms service method of interfacing with the eSPI service.
//
//      To ease this transition, we've split this module into four impl blocks - "common", "new", "legacy", and "routing".
//      Common code should be largely unimpacted by this transition.  The "new" and "legacy" impl blocks provide two different implementations of the same
//      core functionality, one for the new direct async call method and one for the old comms service method.  The "routing" impl block contains code that
//      is responsible for routing messages to the correct implementation based on the type of the incoming message.  When migration is complete, all "legacy"
//      and "routing" functions should be removed and the "new" functions should be renamed to drop the "new" and migrated to the "common" block.
//
//      This approach leads to a bit more code duplication than we'd like during the transition, but should make the final migration a lot simpler and
//      less error-prone, and the transition should be relatively brief.
//

///////// COMMON FUNCTIONS ///////////
impl<RelayHandler: embedded_services::relay::mctp::RelayHandler> Service<RelayHandler> {
    // TODO a lot of the input lifetimes here have to be static because we have a dependency on the comms system, which requires
    //      that everything that talks over it is 'static. Once we eliminate that dependency, we should be able to relax these lifetimes.
    pub async fn init(
        service_storage: &'static OnceLock<Self>,
        mut espi: espi::Espi<'static>,
        relay_handler: RelayHandler,
    ) -> &'static Self {
        espi.wait_for_plat_reset().await;

        let result = service_storage.get_or_init(|| Service {
            endpoint: DEPRECATED_comms::Endpoint::uninit(DEPRECATED_comms::EndpointID::External(
                DEPRECATED_comms::External::Host,
            )),
            espi: Mutex::new(espi),
            host_tx_queue: Channel::new(),
            relay_handler,
        });

        DEPRECATED_comms::register_endpoint(result, &result.endpoint)
            .await
            .unwrap();
        result
    }

    pub(crate) async fn run_service(&self) -> ! {
        let mut espi = self.espi.lock().await;
        loop {
            let event = select(espi.wait_for_event(), self.host_tx_queue.receive()).await;

            match event {
                embassy_futures::select::Either::First(controller_event) => {
                    self.process_controller_event(&mut espi, controller_event)
                        .await
                        .unwrap_or_else(|e| {
                            error!("Critical error processing eSPI controller event: {:?}", e);
                        });
                }
                embassy_futures::select::Either::Second(host_msg) => {
                    self.process_response_to_host_routing(&mut espi, host_msg).await
                }
            }
        }
    }

    // TODO The notification system was not actually used, so this is currently dead code.
    //      We need to implement some interface for triggering notifications from other subsystems, and it may do something like this:
    //
    // async fn process_notification_to_host(&self, espi: &mut espi::Espi<'_>, notification: &NotificationMsg) {
    //     espi.irq_push(notification.offset).await;
    //     info!("espi: Notification id {} sent to Host!", notification.offset);
    // }

    fn write_to_hw(&self, espi: &mut espi::Espi<'static>, packet: &[u8]) -> Result<(), embassy_imxrt::espi::Error> {
        // Send packet via your transport medium
        // SAFETY: Safe as the access to espi is protected by a mut reference.
        let dest_slice = unsafe { espi.oob_get_write_buffer(OOB_PORT_ID)? };
        dest_slice[..packet.len()].copy_from_slice(&packet[..packet.len()]);

        // Write response over OOB
        espi.oob_write_data(OOB_PORT_ID, packet.len() as u8)
    }

    async fn process_controller_event(
        &self,
        espi: &mut espi::Espi<'static>,
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
                    let src_slice =
                        unsafe { slice::from_raw_parts(port_event.base_addr as *const u8, port_event.length) };

                    // TODO: This is a workaround because mctp_rs expects a PEC byte, so we hardcode a 0 at the end.
                    // We should add functionality to mctp_rs to disable PEC.
                    let mut with_pec = [0u8; 100];
                    with_pec[..src_slice.len()].copy_from_slice(src_slice);
                    with_pec[src_slice.len()] = 0;
                    let with_pec = &with_pec[..=src_slice.len()];

                    #[cfg(feature = "defmt")] // Required because without defmt, there is no implementation of UpperHex for [u8]
                    debug!("OOB message: {:02X}", &src_slice[0..]);

                    let mut assembly_buf = [0u8; ASSEMBLY_BUF_SIZE];
                    let mut mctp_ctx = mctp_rs::MctpPacketContext::<SmbusEspiMedium>::new(
                        SmbusEspiMedium,
                        assembly_buf.as_mut_slice(),
                    );

                    match mctp_ctx.deserialize_packet(with_pec) {
                        Ok(Some(message)) => {
                            trace!("MCTP packet successfully deserialized");
                            self.process_request_to_ec_routing(espi, &message, &port_event).await?;
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
}

/////////// ROUTING FUNCTIONS ///////////
impl<RelayHandler: embedded_services::relay::mctp::RelayHandler> Service<RelayHandler> {
    async fn process_response_to_host_routing(
        &self,
        espi: &mut espi::Espi<'static>,
        response: HostResultMessageMigrationEnum<RelayHandler>,
    ) {
        match response {
            HostResultMessageMigrationEnum::Legacy(legacy_msg) => {
                self.process_response_to_host_legacy(espi, legacy_msg).await
            }
            HostResultMessageMigrationEnum::New(new_msg) => self.process_response_to_host_new(espi, new_msg).await,
        }
    }

    async fn process_request_to_ec_routing(
        &self,
        espi: &mut espi::Espi<'static>,
        message: &mctp_rs::MctpMessage<'_, mctp_rs::smbus_espi::SmbusEspiMedium>,
        port_event: &espi::PortEvent,
    ) -> Result<(), Error> {
        if let Ok((header, body)) = message.parse_as::<HostRequest>() {
            self.process_request_to_ec_legacy((header, body), espi, port_event)
                .await
        } else {
            match message.parse_as::<RelayHandler::RequestEnumType>() {
                Ok((header, body)) => self.process_request_to_ec_new((header, body), espi, port_event).await,
                Err(e) => {
                    error!("MCTP ODP type malformed: {:?}", e);
                    espi.complete_port(port_event.port);
                    Err(Error::Serialize)
                }
            }
        }
    }
}

///////////// NEW FUNCTIONS /////////////
impl<RelayHandler: embedded_services::relay::mctp::RelayHandler> Service<RelayHandler> {
    async fn process_request_to_ec_new(
        &self,
        (header, body): (
            <RelayHandler::RequestEnumType as mctp_rs::MctpMessageTrait<'_>>::Header,
            RelayHandler::RequestEnumType,
        ),
        espi: &mut espi::Espi<'static>,
        port_event: &espi::PortEvent,
    ) -> Result<(), Error> {
        use embedded_services::relay::mctp::RelayHeader;
        info!("Host Request received");

        espi.complete_port(port_event.port);

        let response = self.relay_handler.process_request(body).await;
        self.host_tx_queue
            .try_send(HostResultMessageMigrationEnum::New(HostResultMessage {
                handler_service_id: header.get_service_id(),
                message: response,
            }))
            .map_err(|_| Error::Serialize)?;

        Ok(())
    }

    async fn process_response_to_host_new(
        &self,
        espi: &mut espi::Espi<'static>,
        response: HostResultMessage<RelayHandler>,
    ) {
        match self.serialize_packet_from_subsystem_new(espi, response).await {
            Ok(()) => {
                trace!("Full packet successfully sent to host!")
            }
            Err(e) => {
                // TODO we may want to consider sending a failure message to the debug service or something, but that'll require
                //      a 'facility of last resort' on the relay handler, so for now we just log the error
                error!("Packet serialize error {:?}", e);
            }
        }
    }

    async fn serialize_packet_from_subsystem_new(
        &self,
        espi: &mut espi::Espi<'static>,
        result: HostResultMessage<RelayHandler>,
    ) -> Result<(), Error> {
        use embedded_services::relay::mctp::RelayResponse;
        let mut assembly_buf = [0u8; ASSEMBLY_BUF_SIZE];
        let mut mctp_ctx =
            mctp_rs::MctpPacketContext::new(mctp_rs::smbus_espi::SmbusEspiMedium, assembly_buf.as_mut_slice());

        let reply_context: mctp_rs::MctpReplyContext<SmbusEspiMedium> = mctp_rs::MctpReplyContext {
            source_endpoint_id: mctp_rs::EndpointId::Id(0x80),
            destination_endpoint_id: mctp_rs::EndpointId::Id(result.handler_service_id.into()), // TODO We're currently using this incorrectly - it should be the bus address of the host. Revisit once we have assigned a bus address to the host.
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

        let header = result.message.create_header(&result.handler_service_id);
        let mut packet_state = mctp_ctx
            .serialize_packet(reply_context, (header, result.message))
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
            self.process_controller_event(espi, event).await?;
        }
        Ok(())
    }
}

//////////// LEGACY FUNCTIONS ///////////
impl<RelayHandler: embedded_services::relay::mctp::RelayHandler> Service<RelayHandler> {
    async fn process_request_to_ec_legacy(
        &self,
        (header, body): (OdpHeader, HostRequest),
        espi: &mut espi::Espi<'static>,
        port_event: &espi::PortEvent,
    ) -> Result<(), Error> {
        let target_endpoint = header.service.get_endpoint_id();
        trace!(
            "Host Request: Service {:?}, Command {:?}",
            target_endpoint, header.message_id,
        );

        espi.complete_port(port_event.port);
        body.send_to_endpoint(&self.endpoint, target_endpoint)
            .await
            .expect("result error type is infallible");
        info!("MCTP packet forwarded to service: {:?}", target_endpoint);

        Ok(())
    }

    async fn process_response_to_host_legacy(&self, espi: &mut espi::Espi<'static>, response: LegacyHostResultMessage) {
        let source_endpoint = response.source_endpoint;
        match self.serialize_packet_from_subsystem_legacy(espi, response).await {
            Ok(()) => {
                trace!("Full packet successfully sent to host!")
            }
            Err(e) => {
                error!("Packet serialize error {:?}", e);

                self.send_mctp_error_response_legacy(source_endpoint, espi).await;
            }
        }
    }

    async fn send_mctp_error_response_legacy(
        &self,
        endpoint: DEPRECATED_comms::EndpointID,
        espi: &mut espi::Espi<'static>,
    ) {
        let error_msg = LegacyHostResultMessage {
            source_endpoint: endpoint,
            message: HostResult::Debug(Err(debug_service_messages::DebugError::UnspecifiedFailure)),
        };
        self.serialize_packet_from_subsystem_legacy(espi, error_msg)
            .await
            .unwrap_or_else(|_| {
                error!("Critical error reporting MCTP protocol error to host!");
            });
    }

    async fn serialize_packet_from_subsystem_legacy(
        &self,
        espi: &mut espi::Espi<'static>,
        result: LegacyHostResultMessage,
    ) -> Result<(), Error> {
        let mut assembly_buf = [0u8; ASSEMBLY_BUF_SIZE];
        let mut mctp_ctx =
            mctp_rs::MctpPacketContext::new(mctp_rs::smbus_espi::SmbusEspiMedium, assembly_buf.as_mut_slice());

        let source_service: OdpService = OdpService::try_from(result.source_endpoint).map_err(|_| Error::Serialize)?;

        let reply_context: mctp_rs::MctpReplyContext<SmbusEspiMedium> = mctp_rs::MctpReplyContext {
            source_endpoint_id: mctp_rs::EndpointId::Id(0x80),
            destination_endpoint_id: mctp_rs::EndpointId::Id(source_service.into()), // TODO We're currently using this incorrectly - it should be the bus address of the host. Revisit once we have assigned a bus address to the host.
            packet_sequence_number: mctp_rs::MctpSequenceNumber::new(0),
            message_tag: mctp_rs::MctpMessageTag::try_from(3).map_err(|e| {
                error!("serialize_packet_from_subsystem_legacy: {:?}", e);
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
            .serialize_packet(reply_context, (header, result.message))
            .map_err(|e| {
                error!("serialize_packet_from_subsystem_legacy: {:?}", e);
                Error::Serialize
            })?;
        // Send each packet
        while let Some(packet_result) = packet_state.next() {
            let packet = packet_result.map_err(|e| {
                error!("serialize_packet_from_subsystem_legacy: {:?}", e);
                Error::Serialize
            })?;
            // Last byte is PEC, ignore for now
            let packet = &packet[..packet.len() - 1];
            trace!("Sending MCTP response: {:?}", packet);

            self.write_to_hw(espi, packet).map_err(|e| {
                error!("serialize_packet_from_subsystem_legacy: {:?}", e);
                Error::Serialize
            })?;

            // Immediately service the packet with the ESPI HAL
            let event = espi.wait_for_event().await;
            self.process_controller_event(espi, event).await?;
        }
        Ok(())
    }
}

// TODO this impl is also a 'legacy function' and needs to go away when we no longer have services communicating with the eSPI service via the comms service
impl<RelayHandler: embedded_services::relay::mctp::RelayHandler> DEPRECATED_comms::MailboxDelegate
    for Service<RelayHandler>
{
    fn receive(&self, message: &DEPRECATED_comms::Message) -> Result<(), DEPRECATED_comms::MailboxDelegateError> {
        crate::mctp::send_to_comms(message, |source_endpoint, message| {
            debug!("Espi service: recvd response");
            self.host_tx_queue
                .try_send(HostResultMessageMigrationEnum::Legacy(LegacyHostResultMessage {
                    source_endpoint,
                    message,
                }))
                .map_err(|_| DEPRECATED_comms::MailboxDelegateError::BufferFull)
        })
    }
}
