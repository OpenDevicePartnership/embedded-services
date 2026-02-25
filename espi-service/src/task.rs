use crate::Service;

pub async fn espi_service<'hw, R: embedded_services::relay::mctp::RelayHandler>(
    espi_service: &'hw Service<'hw, R>,
) -> Result<embedded_services::Never, crate::espi_service::Error> {
    espi_service.run_service().await
}
