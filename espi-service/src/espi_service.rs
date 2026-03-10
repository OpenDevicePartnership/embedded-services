use core::slice;

use embassy_futures::select::select;
use embassy_imxrt::espi;
use embassy_sync::channel::Channel;
use embassy_sync::mutex::Mutex;
use embedded_services::{GlobalRawMutex, error, info, trace};
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
struct HostResultMessage<RelayHandler: embedded_services::relay::mctp::RelayHandler> {
    pub handler_service_id: RelayHandler::ServiceIdType,
    pub message: RelayHandler::ResultEnumType,
}

#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Error {
    Serialize,
    Buffer(embedded_services::buffer::Error),
}

/// The memory required by the eSPI service to run
pub struct Resources<'hw, RelayHandler: embedded_services::relay::mctp::RelayHandler> {
    inner: Option<ServiceInner<'hw, RelayHandler>>,
}

impl<'hw, RelayHandler: embedded_services::relay::mctp::RelayHandler> Default for Resources<'hw, RelayHandler> {
    fn default() -> Self {
        Self { inner: None }
    }
}

/// Service runner for the eSPI service.  Users must call the run() method on the runner for the service to start processing events.
pub struct Runner<'hw, RelayHandler: embedded_services::relay::mctp::RelayHandler> {
    inner: &'hw ServiceInner<'hw, RelayHandler>,
}

impl<'hw, RelayHandler: embedded_services::relay::mctp::RelayHandler>
    odp_service_common::runnable_service::ServiceRunner<'hw> for Runner<'hw, RelayHandler>
{
    /// Run the service event loop.
    async fn run(self) -> embedded_services::Never {
        self.inner.run().await
    }
}

pub struct Service<'hw, RelayHandler: embedded_services::relay::mctp::RelayHandler> {
    _inner: &'hw ServiceInner<'hw, RelayHandler>,
}

impl<'hw, RelayHandler: embedded_services::relay::mctp::RelayHandler> odp_service_common::runnable_service::Service<'hw>
    for Service<'hw, RelayHandler>
{
    type Resources = Resources<'hw, RelayHandler>;
    type Runner = Runner<'hw, RelayHandler>;
    type ErrorType = core::convert::Infallible;
    type InitParams = InitParams<'hw, RelayHandler>;

    async fn new(
        resources: &'hw mut Self::Resources,
        params: InitParams<'hw, RelayHandler>,
    ) -> Result<(Self, Self::Runner), core::convert::Infallible> {
        let inner = resources.inner.insert(ServiceInner::new(params).await);
        Ok((Self { _inner: inner }, Runner { inner }))
    }
}

pub struct InitParams<'hw, RelayHandler: embedded_services::relay::mctp::RelayHandler> {
    pub espi: espi::Espi<'hw>,
    pub relay_handler: RelayHandler,
}

struct ServiceInner<'hw, RelayHandler: embedded_services::relay::mctp::RelayHandler> {
    espi: Mutex<GlobalRawMutex, espi::Espi<'hw>>,
    host_tx_queue: Channel<GlobalRawMutex, HostResultMessage<RelayHandler>, HOST_TX_QUEUE_SIZE>,
    relay_handler: RelayHandler,
}

impl<'hw, RelayHandler: embedded_services::relay::mctp::RelayHandler> ServiceInner<'hw, RelayHandler> {
    async fn new(mut init_params: InitParams<'hw, RelayHandler>) -> Self {
        init_params.espi.wait_for_plat_reset().await;

        Self {
            espi: Mutex::new(init_params.espi),
            host_tx_queue: Channel::new(),
            relay_handler: init_params.relay_handler,
        }
    }

    async fn run(&self) -> embedded_services::Never {
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
                    self.process_response_to_host(&mut espi, host_msg).await
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

    fn write_to_hw(&self, espi: &mut espi::Espi<'hw>, packet: &[u8]) -> Result<(), embassy_imxrt::espi::Error> {
        // Send packet via your transport medium
        // SAFETY: Safe as the access to espi is protected by a mut reference.
        let dest_slice = unsafe { espi.oob_get_write_buffer(OOB_PORT_ID)? };
        dest_slice[..packet.len()].copy_from_slice(&packet[..packet.len()]);

        // Write response over OOB
        espi.oob_write_data(OOB_PORT_ID, packet.len() as u8)
    }

    async fn process_controller_event(
        &self,
        espi: &mut espi::Espi<'hw>,
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
                    embedded_services::debug!("OOB message: {:02X}", &src_slice[0..]);

                    let mut assembly_buf = [0u8; ASSEMBLY_BUF_SIZE];
                    let mut mctp_ctx = mctp_rs::MctpPacketContext::<SmbusEspiMedium>::new(
                        SmbusEspiMedium,
                        assembly_buf.as_mut_slice(),
                    );

                    match mctp_ctx.deserialize_packet(with_pec) {
                        Ok(Some(message)) => {
                            trace!("MCTP packet successfully deserialized");
                            match message.parse_as::<RelayHandler::RequestEnumType>() {
                                Ok((header, body)) => {
                                    self.process_request_to_ec((header, body), espi, &port_event).await?;
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

    async fn process_request_to_ec(
        &self,
        (header, body): (
            <RelayHandler::RequestEnumType as mctp_rs::MctpMessageTrait<'_>>::Header,
            RelayHandler::RequestEnumType,
        ),
        espi: &mut espi::Espi<'hw>,
        port_event: &espi::PortEvent,
    ) -> Result<(), Error> {
        use embedded_services::relay::mctp::RelayHeader;
        info!("Host Request received");

        espi.complete_port(port_event.port);

        let response = self.relay_handler.process_request(body).await;
        self.host_tx_queue
            .try_send(HostResultMessage {
                handler_service_id: header.get_service_id(),
                message: response,
            })
            .map_err(|_| Error::Serialize)?;

        Ok(())
    }

    async fn process_response_to_host(&self, espi: &mut espi::Espi<'hw>, response: HostResultMessage<RelayHandler>) {
        match self.serialize_packet_from_subsystem(espi, response).await {
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

    async fn serialize_packet_from_subsystem(
        &self,
        espi: &mut espi::Espi<'hw>,
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
