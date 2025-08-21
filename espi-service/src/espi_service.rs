use core::mem::offset_of;
use core::slice;

use core::borrow::{Borrow, BorrowMut};
use embassy_futures::select::select;
use embassy_sync::channel::Channel;
use embassy_sync::mutex::Mutex;
use embassy_sync::once_lock::OnceLock;
use embedded_services::buffer::OwnedRef;
use embedded_services::comms::{self, EndpointID, External, Internal};
use embedded_services::ec_type::message::{AcpiMsgComms, HostMsg, NotificationMsg};
use embedded_services::ec_type::protocols::mctp::{build_mctp_header, handle_mctp_header};
use embedded_services::{GlobalRawMutex, debug, ec_type, error, info};

const HOST_TX_QUEUE_SIZE: usize = 5;

// OOB port number for NXP IMXRT
// REVISIT: When adding support for other platforms, refactor this as they don't have a notion of port IDs
const OOB_PORT_ID: usize = 1;

embedded_services::define_static_buffer!(acpi_buf, u8, [0u8; 69]);

type HostMsgInternal<'a> = (EndpointID, HostMsg<'a>);

pub struct Service<'a, 'b> {
    endpoint: comms::Endpoint,
    ec_memory: Mutex<GlobalRawMutex, &'a mut ec_type::structure::ECMemory>,
    host_tx_queue: Channel<GlobalRawMutex, HostMsgInternal<'b>, HOST_TX_QUEUE_SIZE>,
    acpi_buf_owned_ref: OwnedRef<'a, u8>,
}

impl Service<'_, '_> {
    pub fn new(ec_memory: &'static mut ec_type::structure::ECMemory) -> Self {
        Service {
            endpoint: comms::Endpoint::uninit(EndpointID::External(External::Host)),
            ec_memory: Mutex::new(ec_memory),
            host_tx_queue: Channel::new(),
            acpi_buf_owned_ref: acpi_buf::get_mut().unwrap(),
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

    async fn process_subsystem_msg(&self, espi: &mut espi::Espi<'_>, host_msg: HostMsgInternal<'_>) {
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
        espi: &mut espi::Espi<'_>,
        acpi_response: &AcpiMsgComms<'_>,
        endpoint: EndpointID,
    ) {
        let response_len = acpi_response.payload_len;
        if let Ok((final_packet, final_packet_size)) =
            build_mctp_header(acpi_response.payload.borrow().borrow(), response_len, endpoint)
        {
            debug!("Sending MCTP response: {:?}", &final_packet[..final_packet_size]);

            // SAFETY: Safe as the access to espi is protected by a mut reference.
            let result = unsafe { espi.oob_get_write_buffer(OOB_PORT_ID) };

            match result {
                Ok(dest_slice) => {
                    dest_slice[..final_packet_size].copy_from_slice(&final_packet[..final_packet_size]);
                }
                Err(_e) => {
                    #[cfg(feature = "defmt")]
                    error!("Failed to retrieve OOB write buffer: {}", _e);
                    // TODO: Ask if we need to send a response if the request is malformed
                    return;
                }
            }

            // Write response over OOB
            let res = espi.oob_write_data(OOB_PORT_ID, final_packet_size as u8);

            if res.is_err() {
                #[cfg(feature = "defmt")]
                error!("eSPI OOB write failed: {}", res.err().unwrap());
            }
        } else {
            // Packet malformed, throw it away and respond with ACPI packet with error in reserved field.
            error!("Error building MCTP response packet from service {:?}", endpoint);
            send_mctp_error_response(espi, OOB_PORT_ID).await;
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

                #[cfg(feature = "defmt")]
                debug!("OOB message: {:02X}", &src_slice[0..]);

                let acpi_msg: AcpiMsgComms;
                let endpoint: EndpointID;

                {
                    let mut access = espi_service.acpi_buf_owned_ref.borrow_mut();
                    match handle_mctp_header(src_slice, access.borrow_mut()) {
                        Ok((raw_endpoint, payload_len)) => {
                            acpi_msg = AcpiMsgComms {
                                payload: acpi_buf::get(),
                                payload_len,
                            };
                            endpoint = raw_endpoint;
                        }
                        Err(e) => {
                            // Packet malformed, throw it away and respond with ACPI packet with error in reserved field.
                            error!("MCTP packet malformed: {:?}", e);
                            espi.complete_port(port_event.port);

                            send_mctp_error_response(espi, port_event.port).await;
                            return;
                        }
                    }
                }

                espi.complete_port(port_event.port);
                espi_service.endpoint.send(endpoint, &acpi_msg).await.unwrap();
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

async fn send_mctp_error_response(espi: &mut espi::Espi<'_>, port_id: usize) {
    // SAFETY: Unwrap is safe here as battery will always be supported.
    // Data is ACPI payload [version, instance, reserved (error status), command]
    let (final_packet, final_packet_size) =
        build_mctp_header(&[1, 1, 1, 0], 4, EndpointID::Internal(Internal::Battery)).unwrap();

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
