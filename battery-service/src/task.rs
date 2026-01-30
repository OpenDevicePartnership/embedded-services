use battery_service_messages::DeviceId;
use embedded_services::{comms, error, info};

use crate::{Service, device::Device};

/// Standard dynamic battery data cache
#[derive(Debug, Clone)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum InitError<const N: usize> {
    DeviceRegistrationFailed(heapless::Vec<DeviceId, N>),
    CommsRegistrationFailed,
}

/// Battery service task.
pub async fn task<'a, const N: usize>(
    service: &'static Service,
    devices: [&'static Device; N],
) -> Result<(), InitError<N>> {
    info!("Starting battery-service task");

    let mut failed_devices = heapless::Vec::new();
    for device in devices {
        if service.register_fuel_gauge(device).is_err() {
            error!("Failed to register battery device with DeviceId {:?}", device.id());
            // Panics: Infallible as the Vec is as large as the list of devices passed in.
            failed_devices
                .push(device.id())
                .expect("Infallible as the Vec is as large as the list of devices passed in");
        }
    }

    if failed_devices.is_empty() == false {
        return Err(InitError::DeviceRegistrationFailed(failed_devices));
    }

    if comms::register_endpoint(service, &service.endpoint).await.is_err() {
        error!("Failed to register battery service endpoint");
        return Err(InitError::CommsRegistrationFailed);
    }

    loop {
        service.process_next().await;
    }
}
