use embassy_sync::once_lock::OnceLock;
use embedded_services::{comms, error, info};

use crate::CfuClient;

pub async fn task() {
    info!("Starting cfu client task");
    static CLIENT: OnceLock<CfuClient> = OnceLock::new();
    #[allow(clippy::expect_used)] // panic safety: singleton panic on initialization
    let cfuclient = CLIENT.get_or_init(|| CfuClient::create().expect("cfu client singleton already initialized"));

    if comms::register_endpoint(cfuclient, &cfuclient.tp).await.is_err() {
        error!("Failed to register cfu endpoint");
        return;
    }

    loop {
        if let Err(e) = cfuclient.process_request().await {
            error!("Error processing request: {:?}", e);
        }
    }
}
