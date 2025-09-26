use core::mem::offset_of;
use core::slice;

use core::borrow::{Borrow, BorrowMut};
use embassy_futures::select::select;
use embassy_sync::channel::Channel;
use embassy_sync::mutex::Mutex;
use embassy_sync::once_lock::OnceLock;
use embedded_services::buffer::OwnedRef;
use embedded_services::comms::{self, EndpointID, External, Internal};
use embedded_services::ec_type::message::{HostMsg, HostRequest, NotificationMsg};
use embedded_services::ec_type::protocols::mctp::build_mctp_header;
use embedded_services::{GlobalRawMutex, debug, ec_type, error, info, trace};
use mctp_rs::MctpMessageHeaderAndBody;
use mctp_rs::medium::smbus_espi::{SmbusEspiMedium, SmbusEspiReplyContext};

const HOST_TX_QUEUE_SIZE: usize = 5;

// OOB port number for NXP IMXRT
// REVISIT: When adding support for other platforms, refactor this as they don't have a notion of port IDs
const OOB_PORT_ID: usize = 1;

embedded_services::define_static_buffer!(comms_buf, u8, [0u8; 69]);
embedded_services::define_static_buffer!(assembly_buf, u8, [0u8; 69]);

type HostMsgInternal<'a> = (EndpointID, HostMsg<'a>);

pub struct Service<'a, 'b> {
    endpoint: comms::Endpoint,
    ec_memory: Mutex<GlobalRawMutex, &'a mut ec_type::structure::ECMemory>,
    host_tx_queue: Channel<GlobalRawMutex, HostMsgInternal<'b>, HOST_TX_QUEUE_SIZE>,
    comms_buf_owned_ref: OwnedRef<'a, u8>,
    assembly_buf_owned_ref: OwnedRef<'a, u8>,
}

impl Service<'_, '_> {
    pub fn new(ec_memory: &'static mut ec_type::structure::ECMemory) -> Self {
        Service {
            endpoint: comms::Endpoint::uninit(EndpointID::External(External::Host)),
            ec_memory: Mutex::new(ec_memory),
            host_tx_queue: Channel::new(),
            comms_buf_owned_ref: comms_buf::get_mut().unwrap(),
            assembly_buf_owned_ref: assembly_buf::get_mut().unwrap(),
        }
    }

    async fn route_to_service(&self, offset: usize, length: usize) -> Result<(), ec_type::Error> {
        let mut offset = offset;
        let mut length = length;

        if offset + length > size_of::<ec_type::structure::ECMemory>() {
            return Err(ec_type::Error::InvalidLocation);
        }

        while length > 0 {
            if (offset >= offset_of!(ec_type::structure::ECMemory, ver)
                && offset < offset_of!(ec_type::structure::ECMemory, ver) + size_of::<ec_type::structure::Version>())
                || (offset >= offset_of!(ec_type::structure::ECMemory, caps)
                    && offset
                        < offset_of!(ec_type::structure::ECMemory, caps)
                            + size_of::<ec_type::structure::Capabilities>())
            {
                // This is a read-only section. eSPI master should not write to it.
                return Err(ec_type::Error::InvalidLocation);
            } else if offset >= offset_of!(ec_type::structure::ECMemory, batt)
                && offset < offset_of!(ec_type::structure::ECMemory, batt) + size_of::<ec_type::structure::Battery>()
            {
                self.route_to_battery_service(&mut offset, &mut length).await?;
            } else if offset >= offset_of!(ec_type::structure::ECMemory, therm)
                && offset < offset_of!(ec_type::structure::ECMemory, therm) + size_of::<ec_type::structure::Thermal>()
            {
                self.route_to_thermal_service(&mut offset, &mut length).await?;
            } else if offset >= offset_of!(ec_type::structure::ECMemory, alarm)
                && offset < offset_of!(ec_type::structure::ECMemory, alarm) + size_of::<ec_type::structure::TimeAlarm>()
            {
                self.route_to_time_alarm_service(&mut offset, &mut length).await?;
            }
        }

        Ok(())
    }

    async fn route_to_battery_service(&self, offset: &mut usize, length: &mut usize) -> Result<(), ec_type::Error> {
        let msg = {
            let memory_map = self
                .ec_memory
                .try_lock()
                .expect("Messages handled one after another, should be infallible.");
            ec_type::mem_map_to_battery_msg(&memory_map, offset, length)?
        };

        comms::send(
            EndpointID::External(External::Host),
            EndpointID::Internal(Internal::Battery),
            &msg,
        )
        .await
        .unwrap();

        Ok(())
    }

    async fn route_to_thermal_service(&self, offset: &mut usize, length: &mut usize) -> Result<(), ec_type::Error> {
        let msg = {
            let memory_map = self
                .ec_memory
                .try_lock()
                .expect("Messages handled one after another, should be infallible.");
            ec_type::mem_map_to_thermal_msg(&memory_map, offset, length)?
        };

        comms::send(
            EndpointID::External(External::Host),
            EndpointID::Internal(Internal::Thermal),
            &msg,
        )
        .await
        .unwrap();

        Ok(())
    }

    async fn route_to_time_alarm_service(&self, offset: &mut usize, length: &mut usize) -> Result<(), ec_type::Error> {
        let msg = {
            let memory_map = self
                .ec_memory
                .try_lock()
                .expect("Messages handled one after another, should be infallible.");
            ec_type::mem_map_to_time_alarm_msg(&memory_map, offset, length)?
        };

        comms::send(
            EndpointID::External(External::Host),
            EndpointID::Internal(Internal::TimeAlarm),
            &msg,
        )
        .await
        .unwrap();

        Ok(())
    }

    async fn wait_for_subsystem_msg(&self) -> HostMsgInternal<'_> {
        self.host_tx_queue.receive().await
    }

    async fn process_subsystem_msg(&self, espi: &mut espi::Espi<'static>, host_msg: HostMsgInternal<'_>) {
        let (endpoint, host_msg) = host_msg;
        match host_msg {
            HostMsg::Notification(notification_msg) => self.process_notification_to_host(espi, &notification_msg).await,
            HostMsg::Response(acpi_msg_comms) => self.process_response_to_host(espi, &acpi_msg_comms, endpoint).await,
        }
    }

    async fn process_notification_to_host(&self, espi: &mut espi::Espi<'_>, notification: &NotificationMsg) {
        espi.irq_push(notification.offset).await;
        info!("espi: Notification id {} sent to Host!", notification.offset);
    }

    async fn process_response_to_host(
        &self,
        espi: &mut espi::Espi<'static>,
        acpi_response: &HostRequest<'_>,
        endpoint: EndpointID,
    ) {
        let mut pkt_ctx_buf = [0u8; 160];
        let response_len = acpi_response.payload_len;
        let access = acpi_response.payload.borrow();
        let acpi_pkt: &[u8] = access.borrow();

        let mut mctp_ctx =
            mctp_rs::MctpPacketContext::new(mctp_rs::medium::smbus_espi::SmbusEspiMedium, &mut pkt_ctx_buf);

        let reply_context: mctp_rs::MctpReplyContext<SmbusEspiMedium> = mctp_rs::MctpReplyContext {
            source_endpoint_id: mctp_rs::endpoint_id::EndpointId::Id(0x80),
            destination_endpoint_id: match endpoint {
                EndpointID::Internal(Internal::Battery) => mctp_rs::endpoint_id::EndpointId::Id(8),
                EndpointID::Internal(Internal::Thermal) => mctp_rs::endpoint_id::EndpointId::Id(9),
                EndpointID::Internal(Internal::Debug) => mctp_rs::endpoint_id::EndpointId::Id(10),
                _ => mctp_rs::endpoint_id::EndpointId::Id(0x80),
            },
            packet_sequence_number: mctp_rs::mctp_sequence_number::MctpSequenceNumber::new(0),
            message_tag: mctp_rs::MctpMessageTag::try_from(3).unwrap(),
            medium_context: SmbusEspiReplyContext {
                destination_slave_address: 1,
                source_slave_address: 0,
            }, // Medium-specific context
        };

        let mut packet_state = mctp_ctx
            .serialize_packet(reply_context, &acpi_pkt[..response_len])
            .unwrap();
        // Send each packet
        while let Some(packet_result) = packet_state.next() {
            let packet = packet_result.unwrap();
            // Last byte is PEC, ignore for now
            trace!("Sending MCTP response: {:?}", &packet[..packet.len() - 1]);

            // Send packet via your transport medium
            // SAFETY: Safe as the access to espi is protected by a mut reference.
            let result = unsafe { espi.oob_get_write_buffer(OOB_PORT_ID) };

            match result {
                Ok(dest_slice) => {
                    dest_slice[..packet.len() - 1].copy_from_slice(&packet[..packet.len() - 1]);
                }
                Err(_e) => {
                    #[cfg(feature = "defmt")]
                    error!("Failed to retrieve OOB write buffer: {}", _e);
                    // TODO: Ask if we need to send a response if the request is malformed
                    return;
                }
            }

            // Write response over OOB
            let res = espi.oob_write_data(OOB_PORT_ID, (packet.len() - 1) as u8);

            if res.is_err() {
                #[cfg(feature = "defmt")]
                error!("eSPI OOB write failed: {}", res.err().unwrap());
                return;
            }

            // Immediately service the packet with the ESPI HAL
            // REVISIT: Can i just do espi.complete_port(OOB_PORT_ID);
            // or is there more business logic in wait_for_event();
            let event = espi.wait_for_event().await;
            process_controller_event(espi, self, event).await;
        }
    }
}

impl comms::MailboxDelegate for Service<'_, '_> {
    fn receive(&self, message: &comms::Message) -> Result<(), comms::MailboxDelegateError> {
        if let Some(msg) = message.data.get::<HostMsg>() {
            let host_msg = (message.from, msg.clone());
            debug!("Espi service: recvd acpi response");
            if self.host_tx_queue.try_send(host_msg).is_err() {
                return Err(comms::MailboxDelegateError::BufferFull);
            }
        } else {
            let mut memory_map = self
                .ec_memory
                .try_lock()
                .expect("Messages handled one after another, should be infallible.");
            if let Some(msg) = message.data.get::<ec_type::message::CapabilitiesMessage>() {
                ec_type::update_capabilities_section(msg, &mut memory_map);
            } else if let Some(msg) = message.data.get::<ec_type::message::BatteryMessage>() {
                ec_type::update_battery_section(msg, &mut memory_map);
            } else if let Some(msg) = message.data.get::<ec_type::message::ThermalMessage>() {
                ec_type::update_thermal_section(msg, &mut memory_map);
            } else if let Some(msg) = message.data.get::<ec_type::message::TimeAlarmMessage>() {
                ec_type::update_time_alarm_section(msg, &mut memory_map);
            } else {
                return Err(comms::MailboxDelegateError::MessageNotFound);
            }
        }

        Ok(())
    }
}

static ESPI_SERVICE: OnceLock<Service> = OnceLock::new();

use embassy_imxrt::espi;

#[embassy_executor::task]
pub async fn espi_service(mut espi: espi::Espi<'static>, memory_map_buffer: &'static mut [u8]) {
    info!("Reserved eSPI memory map buffer size: {}", memory_map_buffer.len());
    info!("eSPI MemoryMap size: {}", size_of::<ec_type::structure::ECMemory>());

    if size_of::<ec_type::structure::ECMemory>() > memory_map_buffer.len() {
        panic!("eSPI MemoryMap is too big for reserved memory buffer!!!");
    }

    memory_map_buffer.fill(0);

    let memory_map: &mut ec_type::structure::ECMemory =
        unsafe { &mut *(memory_map_buffer.as_mut_ptr() as *mut ec_type::structure::ECMemory) };

    espi.wait_for_plat_reset().await;

    info!("Initializing memory map");
    memory_map.ver.major = ec_type::structure::EC_MEMMAP_VERSION.major;
    memory_map.ver.minor = ec_type::structure::EC_MEMMAP_VERSION.minor;
    memory_map.ver.spin = ec_type::structure::EC_MEMMAP_VERSION.spin;
    memory_map.ver.res0 = ec_type::structure::EC_MEMMAP_VERSION.res0;

    let espi_service = ESPI_SERVICE.get_or_init(|| Service::new(memory_map));
    comms::register_endpoint(espi_service, &espi_service.endpoint)
        .await
        .unwrap();

    loop {
        let event = select(espi.wait_for_event(), espi_service.wait_for_subsystem_msg()).await;

        match event {
            embassy_futures::select::Either::First(controller_event) => {
                process_controller_event(&mut espi, espi_service, controller_event).await
            }
            embassy_futures::select::Either::Second(host_msg) => {
                espi_service.process_subsystem_msg(&mut espi, host_msg).await
            }
        }
    }
}

async fn process_controller_event(
    espi: &mut espi::Espi<'static>,
    espi_service: &Service<'_, '_>,
    event: Result<embassy_imxrt::espi::Event, embassy_imxrt::espi::Error>,
) {
    match event {
        Ok(espi::Event::PeripheralEvent(port_event)) => {
            info!(
                "eSPI PeripheralEvent Port: {}, direction: {}, address: {}, offset: {}, length: {}",
                port_event.port, port_event.direction, port_event.offset, port_event.base_addr, port_event.length,
            );

            // If it is a peripheral channel write, then we need to notify the service
            if port_event.direction {
                let res = espi_service
                    .route_to_service(port_event.offset, port_event.length)
                    .await;

                if res.is_err() {
                    error!(
                        "eSPI master send invalid offset: {} length: {}",
                        port_event.offset, port_event.length
                    );
                }
            }

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

                #[cfg(feature = "defmt")]
                debug!("OOB message: {:02X}", &src_slice[0..]);

                let host_request: HostRequest;
                let endpoint: EndpointID;

                {
                    let mut assembly_access = espi_service.assembly_buf_owned_ref.borrow_mut();
                    let mut comms_access = espi_service.comms_buf_owned_ref.borrow_mut();
                    let mut mctp_ctx = mctp_rs::MctpPacketContext::<SmbusEspiMedium>::new(
                        SmbusEspiMedium,
                        assembly_access.borrow_mut(),
                    );

                    match mctp_ctx.deserialize_packet(with_pec) {
                        Ok(Some(message)) => {
                            debug!("Complete message received");

                            match message.parse_as::<mctp_rs::message_type::odp::Odp>() {
                                Ok((one, two)) => return,
                                Err(e) => return,
                            }

                            // Complete message received
                            match message.header_and_body {
                                MctpMessageHeaderAndBody::Odp { header, body } => {
                                    info!("Body {:?}", body);
                                    let buf: &mut [u8] = comms_access.borrow_mut();
                                    buf[..body.len()].copy_from_slice(body);
                                    endpoint = match u8::from(message.reply_context.destination_endpoint_id) {
                                        8 => EndpointID::Internal(embedded_services::comms::Internal::Battery),
                                        9 => EndpointID::Internal(embedded_services::comms::Internal::Thermal),
                                        10 => EndpointID::Internal(embedded_services::comms::Internal::Debug),
                                        _ => unimplemented!(),
                                    };
                                    host_request = HostRequest {
                                        payload: comms_buf::get(),
                                        payload_len: body.len(),
                                        command: header.command_code.into(),
                                        status: header.completion_code.into(),
                                    };
                                    info!(
                                        "Host Request {:?} {:?} {:?}",
                                        host_request.payload_len, host_request.command, host_request.status,
                                    );
                                }
                                _ => todo!(),
                            }
                        }
                        Ok(None) => {
                            // Partial message, waiting for more packets
                            error!("Partial msg, should not happen");
                            unreachable!()
                        }
                        Err(_e) => {
                            // Handle protocol or medium error
                            error!("MCTP packet malformed");
                            espi.complete_port(port_event.port);

                            send_mctp_error_response(espi, port_event.port);
                            return;
                        }
                    }
                }

                espi.complete_port(port_event.port);
                espi_service.endpoint.send(endpoint, &host_request).await.unwrap();
                info!("MCTP packet forwarded to service: {:?}", endpoint);
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
}

fn send_mctp_error_response(espi: &mut espi::Espi<'_>, port_id: usize) {
    // SAFETY: Unwrap is safe here as battery will always be supported.
    // Data is ACPI payload [version, instance, reserved (error status), command]
    let (final_packet, final_packet_size) =
        build_mctp_header(&[1, 1, 1, 0], 4, EndpointID::Internal(Internal::Battery), true, true)
            .expect("Unexpected error building MCTP header");

    let result = unsafe { espi.oob_get_write_buffer(port_id) };
    match result {
        Ok(dest_slice) => {
            dest_slice[..final_packet_size].copy_from_slice(&final_packet[..final_packet_size]);
        }
        Err(_e) => {
            #[cfg(feature = "defmt")]
            error!("Failed to retrieve OOB write buffer: {}", _e);
            return;
        }
    }

    // Write response over OOB
    let res = espi.oob_write_data(port_id, final_packet_size as u8);
    if res.is_err() {
        #[cfg(feature = "defmt")]
        error!("eSPI OOB write failed: {}", res.err().unwrap());
    }
}
