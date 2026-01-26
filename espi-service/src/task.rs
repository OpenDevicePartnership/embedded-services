use embassy_futures::select::select;
use embassy_imxrt::espi;
use embedded_services::comms;

use crate::{ESPI_SERVICE, Service, process_controller_event};

pub async fn espi_service(
    mut espi: espi::Espi<'static>,
) -> Result<embedded_services::Never, crate::espi_service::Error> {
    espi.wait_for_plat_reset().await;

    let espi_service = ESPI_SERVICE.get_or_init(|| Service::new());
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
