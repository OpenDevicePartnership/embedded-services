use crate::Service;

// TODO: We currently require that the service has lifetime 'static because we still communicate with
//       some services over the legacy comms system, which requires that things that interact with it
//       have lifetime 'static.  Once we've fully transitioned to the direct async call method of interfacing
//       between services, we should be able to relax this requirement to just require that the service has
//       the same lifetime as the services it's communicating with.
pub async fn espi_service<R: embedded_services::relay::mctp::RelayHandler>(
    espi_service: &'static Service<R>,
) -> Result<embedded_services::Never, crate::espi_service::Error> {
    espi_service.run_service().await
}
