use core::mem::offset_of;
use core::slice;

use core::borrow::{Borrow, BorrowMut};
use embassy_sync::mutex::Mutex;
use embassy_sync::once_lock::OnceLock;
use embassy_sync::signal::Signal;
use embedded_services::comms::{self, EndpointID, External, Internal};
use embedded_services::ec_type::message::AcpiMsgComms;
use embedded_services::ec_type::protocols::mctp::{build_mctp_header, handle_mctp_header};
use embedded_services::{GlobalRawMutex, ec_type, error, info};

pub struct Service<'a, 'b> {
    endpoint: comms::Endpoint,
    ec_memory: Mutex<GlobalRawMutex, &'a mut ec_type::structure::ECMemory>,
    comms_signal: Signal<GlobalRawMutex, AcpiMsgComms<'b>>,
}

impl Service<'_, '_> {
    pub fn new(ec_memory: &'static mut ec_type::structure::ECMemory) -> Self {
        Service {
            endpoint: comms::Endpoint::uninit(EndpointID::External(External::Host)),
            ec_memory: Mutex::new(ec_memory),
            comms_signal: Signal::new(),
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
}

impl comms::MailboxDelegate for Service<'_, '_> {
    fn receive(&self, message: &comms::Message) -> Result<(), comms::MailboxDelegateError> {
        if let Some(msg) = message.data.get::<AcpiMsgComms>() {
            info!("Espi service: recvd acpi response");
            self.comms_signal.signal(msg.clone());
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

    embedded_services::define_static_buffer!(acpi_buf, u8, [0u8; 69]);

    let owned_ref = acpi_buf::get_mut().unwrap();

    loop {
        let event = espi.wait_for_event().await;
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

                espi.complete_port(port_event.port).await;
            }
            Ok(espi::Event::OOBEvent(port_event)) => {
                info!(
                    "eSPI OOBEvent Port: {}, direction: {}, address: {}, offset: {}, length: {}",
                    port_event.port, port_event.direction, port_event.offset, port_event.base_addr, port_event.length,
                );

                if port_event.direction {
                    let src_slice =
                        unsafe { slice::from_raw_parts(port_event.base_addr as *const u8, port_event.length) };

                    #[cfg(feature = "defmt")]
                    info!("OOB message: {:02X}", &src_slice[0..]);

                    let acpi_msg: AcpiMsgComms;

                    {
                        let mut access = owned_ref.borrow_mut();
                        match handle_mctp_header(src_slice, access.borrow_mut()) {
                            Ok((endpoint, payload_len)) => {
                                acpi_msg = AcpiMsgComms {
                                    payload: acpi_buf::get(),
                                    payload_len,
                                    endpoint,
                                };
                            }
                            Err(e) => {
                                // Packet malformed, throw it away
                                error!("MCTP packet malformed: {:?}", e);
                                continue;
                            }
                        }
                    }

                    espi_service.endpoint.send(acpi_msg.endpoint, &acpi_msg).await.unwrap();
                    info!("MCTP packet forwarded to service: {:?}", acpi_msg.endpoint);

                    let acpi_response = espi_service.comms_signal.wait().await;

                    let response_len = acpi_response.payload_len;
                    let endpoint = acpi_response.endpoint;
                    if let Ok((final_packet, final_packet_size)) =
                        build_mctp_header(acpi_response.payload.borrow().borrow(), response_len, endpoint)
                    {
                        info!("Sending MCTP response: {:?}", &final_packet[..final_packet_size]);

                        let result = unsafe { espi.oob_get_write_buffer(port_event.port) };

                        match result {
                            Ok(dest_slice) => {
                                dest_slice[..final_packet_size].copy_from_slice(&final_packet[..final_packet_size]);
                            }
                            Err(_e) => {
                                #[cfg(feature = "defmt")]
                                error!("Failed to retrieve OOB write buffer: {}", _e);
                                espi.complete_port(port_event.port).await;
                                continue;
                            }
                        }

                        // Don't complete event until we read out OOB data
                        espi.complete_port(port_event.port).await;

                        // Test code send same data on loopback
                        let res = espi.oob_write_data(port_event.port, final_packet_size as u8);

                        if res.is_err() {
                            #[cfg(feature = "defmt")]
                            error!("eSPI OOB write failed: {}", res.err().unwrap());
                        }
                    } else {
                        #[cfg(feature = "defmt")]
                        error!("Error building MCTP response packet from service {:?}", endpoint);
                    }
                } else {
                    espi.complete_port(port_event.port).await;
                }
            }
            Ok(espi::Event::Port80) => {
                info!("eSPI Port 80");
            }
            Ok(espi::Event::WireChange(_)) => {
                info!("eSPI WireChange");
            }
            Err(_) => {
                error!("eSPI Failed");
            }
        }
    }
}
