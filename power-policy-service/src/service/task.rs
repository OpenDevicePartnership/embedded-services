use embedded_services::{comms, error, event::Receiver, info, sync::Lockable};

use crate::psu::{Psu, event::RequestData};

use super::Service;

#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum InitError {
    /// Comms registration failed
    RegistrationFailed,
    /// Power device registration failed
    PowerDeviceRegistrationFailed,
    /// Charger device registration failed
    ChargerDeviceRegistrationFailed,
}

/// Runs the power policy task.
pub async fn task<D: Lockable + 'static, R: Receiver<RequestData> + 'static>(
    policy: &'static Service<'static, D, R>,
) -> Result<embedded_services::Never, InitError>
where
    D::Inner: Psu,
{
    info!("Starting power policy task");
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
