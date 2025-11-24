use embassy_sync::once_lock::OnceLock;
use embedded_services::{comms, error, info};

use crate::{PowerPolicy, config};

pub async fn task(config: config::Config) {
    info!("Starting power policy task");
    static POLICY: OnceLock<PowerPolicy> = OnceLock::new();
    let policy =
        POLICY.get_or_init(|| PowerPolicy::create(config).expect("Power policy singleton already initialized"));

    if comms::register_endpoint(policy, &policy.tp).await.is_err() {
        error!("Failed to register power policy endpoint");
        return;
    }

    loop {
        if let Err(e) = policy.process().await {
            error!("Error processing request: {:?}", e);
        }
    }
}
