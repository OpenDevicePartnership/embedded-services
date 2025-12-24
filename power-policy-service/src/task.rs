use embassy_sync::once_lock::OnceLock;
use embedded_services::{comms, error, info};

use crate::{PowerPolicy, config};

#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum InitError {
    /// Power policy singleton has already been initialized
    AlreadyInitialized,
    /// Comms registration failed
    RegistrationFailed,
}

pub async fn task(config: config::Config) -> Result<embedded_services::Never, InitError> {
    info!("Starting power policy task");
    static POLICY: OnceLock<PowerPolicy> = OnceLock::new();
    let policy = if let Some(policy) = PowerPolicy::create(config) {
        POLICY.get_or_init(|| policy)
    } else {
        error!("Power policy service already initialized");
        return Err(InitError::AlreadyInitialized);
    };

    if comms::register_endpoint(policy, &policy.tp).await.is_err() {
        error!("Failed to register power policy endpoint");
        return Err(InitError::RegistrationFailed);
    }

    loop {
        if let Err(e) = policy.process().await {
            error!("Error processing request: {:?}", e);
        }
    }
}
