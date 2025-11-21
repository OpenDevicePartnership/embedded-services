use embedded_services::{comms, error, info};

use crate::SERVICE;

/// Battery service task.
pub async fn task() {
    info!("Starting battery-service task");

    if comms::register_endpoint(&SERVICE, &SERVICE.endpoint).await.is_err() {
        error!("Failed to register battery service endpoint");
        return;
    }

    loop {
        SERVICE.process_next().await;
    }
}
