use crate::{AcpiTimerId, Service};
use embedded_services::info;

// TODO This pattern of pushing task declaration to the 'application' level allows us to
//      avoid bringing in the async executor dependency into the service crate itself, but
//      it also means that we can't really construct and start a task in a single motion,
//      which means it's possible to forget to start a task after constructing the service
//      and have the service misbehave.  Investigate ways to improve that to make the API
//      less error-prone.

/// Call this from a dedicated async task.  Must be called exactly once per service.
pub async fn command_handler_task(service: &'static Service) {
    info!("Starting time-alarm service task");
    service.handle_requests().await;
}

/// Call this from a dedicated async task.  Must be called exactly once per service.
pub async fn ac_timer_task(service: &'static Service) {
    info!("Starting time-alarm timer task");
    service.handle_timer(AcpiTimerId::AcPower).await;
}

/// Call this from a dedicated async task.  Must be called exactly once per service.
pub async fn dc_timer_task(service: &'static Service) {
    info!("Starting time-alarm timer task");
    service.handle_timer(AcpiTimerId::DcPower).await;
}
