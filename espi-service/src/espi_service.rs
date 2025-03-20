use core::cell::RefCell;
use core::mem::offset_of;

use embassy_sync::once_lock::OnceLock;
use embedded_services::comms::{self, EndpointID, External, Internal};
use embedded_services::{ec_type, error, info};

pub struct Service<'a> {
    endpoint: comms::Endpoint,
    ec_memory: RefCell<&'a mut ec_type::structure::ECMemory>,
}

impl Service<'_> {
    pub fn new(ec_memory: &'static mut ec_type::structure::ECMemory) -> Self {
        Service {
            endpoint: comms::Endpoint::uninit(EndpointID::External(External::Host)),
            ec_memory: RefCell::new(ec_memory),
        }
    }

    fn update_battery_section(&self, msg: &ec_type::message::BatteryMessage) {
        let mut memory_map = self.ec_memory.borrow_mut();
        ec_type::update_battery_section(msg, &mut memory_map);
    }

    fn update_capabilities_section(&self, msg: &ec_type::message::CapabilitiesMessage) {
        let mut memory_map = self.ec_memory.borrow_mut();
        ec_type::update_capabilities_section(msg, &mut memory_map);
    }

    fn update_thermal_section(&self, msg: &ec_type::message::ThermalMessage) {
        let mut memory_map = self.ec_memory.borrow_mut();
        ec_type::update_thermal_section(msg, &mut memory_map);
    }

    fn update_time_alarm_section(&self, msg: &ec_type::message::TimeAlarmMessage) {
        let mut memory_map = self.ec_memory.borrow_mut();
        ec_type::update_time_alarm_section(msg, &mut memory_map);
    }

    async fn route_to_service(&self, offset: usize, length: usize) -> Result<(), ec_type::Error> {
        let mut offset = offset;
        let mut length = length;

        if offset + length > size_of::<ec_type::structure::ECMemory>() {
            return Err(ec_type::Error::InvalidLocation);
        }

        while length > 0 {
            if offset >= offset_of!(ec_type::structure::ECMemory, ver)
                && offset < offset_of!(ec_type::structure::ECMemory, ver) + size_of::<ec_type::structure::Version>()
            {
                // This is a read-only section. eSPI master should not write to it.
                return Err(ec_type::Error::InvalidLocation);
            } else if offset >= offset_of!(ec_type::structure::ECMemory, caps)
                && offset
                    < offset_of!(ec_type::structure::ECMemory, caps) + size_of::<ec_type::structure::Capabilities>()
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
            let memory_map = self.ec_memory.borrow();
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
            let memory_map = self.ec_memory.borrow();
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
            let memory_map = self.ec_memory.borrow();
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

impl comms::MailboxDelegate for Service<'_> {
    fn receive(&self, message: &comms::Message) -> Result<(), comms::MailboxDelegateError> {
        if let Some(msg) = message.data.get::<ec_type::message::CapabilitiesMessage>() {
            self.update_capabilities_section(msg);
        } else if let Some(msg) = message.data.get::<ec_type::message::BatteryMessage>() {
            self.update_battery_section(msg);
        } else if let Some(msg) = message.data.get::<ec_type::message::ThermalMessage>() {
            self.update_thermal_section(msg);
        } else if let Some(msg) = message.data.get::<ec_type::message::TimeAlarmMessage>() {
            self.update_time_alarm_section(msg);
        } else {
            return Err(comms::MailboxDelegateError::MessageNotFound);
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
        embassy_time::Timer::after_secs(10).await;

        let event = espi.wait_for_event().await;
        match event {
            Ok(espi::Event::Port0(port_event)) => {
                info!(
                    "eSPI Port 0, direction: {}, offset: {}, length: {}",
                    port_event.direction, port_event.offset, port_event.length,
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

                espi.complete_port(0).await;
            }
            Ok(espi::Event::Port1(_)) => {
                info!("eSPI Port 1");
            }
            Ok(espi::Event::Port2(_port_event)) => {
                info!("eSPI Port 2");
            }
            Ok(espi::Event::Port3(_)) => {
                info!("eSPI Port 3");
            }
            Ok(espi::Event::Port4(_)) => {
                info!("eSPI Port 4");
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
