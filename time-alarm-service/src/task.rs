use crate::{AcpiTimerId, Service};
use embedded_services::info;

/// Call this from a dedicated async task.  Must be called exactly once per service.
pub async fn command_handler_task(service: &'static Service) {
    info!("Starting time-alarm service task");
    service.handle_requests().await;
}

/// Call this from a dedicated async task.  Must be called exactly once per service.
pub async fn ac_timer_task(service: &'static Service) {
    info!("Starting time-alarm AC timer task");
    service.handle_timer(AcpiTimerId::AcPower).await;
}

/// Call this from a dedicated async task.  Must be called exactly once per service.
pub async fn dc_timer_task(service: &'static Service) {
    info!("Starting time-alarm DC timer task");
    service.handle_timer(AcpiTimerId::DcPower).await;
}
