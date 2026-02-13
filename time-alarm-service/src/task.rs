use crate::Service;
use embedded_services::info;

/// Call this from a dedicated async task.  Must be called exactly once on each service instance.
/// Note that on-device, 'hw must be 'static.  We're generic over 'hw to enable some test scenarios leveraging tokio and mocks.
pub async fn run_service<'hw>(service: &'hw Service<'hw>) -> ! {
    info!("Starting time-alarm service task");
    service.run_service().await
}
