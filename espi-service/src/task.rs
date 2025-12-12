use embassy_futures::select::select;
use embassy_imxrt::espi;
use embedded_services::{comms, ec_type, info};

use crate::{ESPI_SERVICE, Service, process_controller_event};

pub async fn espi_service(
    mut espi: espi::Espi<'static>,
    memory_map_buffer: &'static mut [u8],
) -> Result<embedded_services::Never, crate::espi_service::Error> {
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
    comms::register_endpoint(espi_service, espi_service.endpoint())
        .await
        .unwrap();

    loop {
        let event = select(espi.wait_for_event(), espi_service.wait_for_response()).await;

        match event {
            embassy_futures::select::Either::First(controller_event) => {
                process_controller_event(&mut espi, espi_service, controller_event).await?
            }
            embassy_futures::select::Either::Second(host_msg) => {
                espi_service.process_response_to_host(&mut espi, host_msg).await
            }
        }
    }
}
