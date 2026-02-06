use embedded_services::{error, info};

use crate::CfuClient;

pub async fn task(cfu_client: &'static CfuClient) {
    info!("Starting cfu client task");

    loop {
        if let Err(e) = cfu_client.process_request().await {
            error!("Error processing request: {:?}", e);
        }
    }
}
