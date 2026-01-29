use embedded_services::{
    comms, error, info,
    power::policy::{charger, device},
};

use crate::PowerPolicy;

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

/// Initializes and runs the power policy task.
///
/// This task function initializes the power policy service by registering its endpoint
/// with the comms layer, registering any provided **non Type-C** power devices and charger devices,
/// and then continuously processes incoming power policy requests. It should be run as its own task
/// that never returns.
///
/// # Generic Parameters
///
/// * `POLICY_CHANNEL_SIZE` - The capacity of the channel used for power policy messages.
/// * `NUM_POWER_DEVICES` - The number of **non Type-C** power devices to be managed by power policy.
/// * `NUM_CHARGERS` - The number of charger devices to be managed by power policy.
///
/// # Arguments
///
/// * `policy` - A static reference to the [`PowerPolicy`] instance that manages power policies.
/// * `power_devices` - An optional array of static references to **non Type-C** power device containers.
///   If provided, each device will be registered with the policy context.
/// * `charger_devices` - An optional array of static references to charger device containers.
///   If provided, each charger will be registered with the policy context.
///
/// # Returns
///
/// Returns `Result<embedded_services::Never, InitError>`. The `Never` type indicates that
/// this function runs indefinitely once initialized. The function returns an error if
/// initialization fails at any stage:
/// - [`InitError::RegistrationFailed`] - if comms endpoint registration fails
/// - [`InitError::PowerDeviceRegistrationFailed`] - if power device registration fails
/// - [`InitError::ChargerDeviceRegistrationFailed`] - if charger device registration fails
pub async fn task<const POLICY_CHANNEL_SIZE: usize, const NUM_POWER_DEVICES: usize, const NUM_CHARGERS: usize>(
    policy: &'static PowerPolicy<POLICY_CHANNEL_SIZE>,
    power_devices: Option<[&'static impl device::DeviceContainer<POLICY_CHANNEL_SIZE>; NUM_POWER_DEVICES]>,
    charger_devices: Option<[&'static impl charger::ChargerContainer; NUM_CHARGERS]>,
) -> Result<embedded_services::Never, InitError> {
    info!("Starting power policy task");
    if comms::register_endpoint(policy, &policy.tp).await.is_err() {
        error!("Failed to register power policy endpoint");
        return Err(InitError::RegistrationFailed);
    }

    if let Some(power_devices) = power_devices {
        for device in power_devices {
            policy
                .context
                .register_device(device)
                .map_err(|_| InitError::PowerDeviceRegistrationFailed)?;
        }
    }

    if let Some(charger_devices) = charger_devices {
        for device in charger_devices {
            policy
                .context
                .register_charger(device)
                .map_err(|_| InitError::ChargerDeviceRegistrationFailed)?;
        }
    }

    loop {
        if let Err(e) = policy.process().await {
            error!("Error processing request: {:?}", e);
        }
    }
}
