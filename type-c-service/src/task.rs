use core::future::Future;
use embedded_services::{error, event, info, sync::Lockable};

use power_policy_interface::psu;

use crate::{service::Service, wrapper::ControllerWrapper};

/// Task to run the Type-C service, takes a closure to customize the event loop
pub async fn task_closure<
    'a,
    M,
    D,
    PSU: Lockable,
    S,
    V,
    Fut: Future<Output = ()>,
    F: Fn(&'a Service<'a, PSU>) -> Fut,
    const N: usize,
>(
    service: &'static Service<'a, PSU>,
    wrappers: [&'a ControllerWrapper<'a, M, D, S, V>; N],
    cfu_client: &'a cfu_service::CfuClient,
    f: F,
) where
    M: embassy_sync::blocking_mutex::raw::RawMutex,
    D: Lockable,
    PSU::Inner: psu::Psu,
    S: event::Sender<power_policy_interface::psu::event::EventData>,
    V: crate::wrapper::FwOfferValidator,
    D::Inner: crate::type_c::controller::Controller,
{
    info!("Starting type-c task");

    // TODO: move this service to use the new power policy event subscribers and receivers
    // See https://github.com/OpenDevicePartnership/embedded-services/issues/742

    for controller_wrapper in wrappers {
        if controller_wrapper.register(service.controllers(), cfu_client).is_err() {
            error!("Failed to register a controller");
            return;
        }
    }

    loop {
        f(service).await;
    }
}

/// Task to run the Type-C service, running the default event loop
pub async fn task<'a, M, D, PSU: Lockable, S, V, const N: usize>(
    service: &'static Service<'a, PSU>,
    wrappers: [&'a ControllerWrapper<'a, M, D, S, V>; N],
    cfu_client: &'a cfu_service::CfuClient,
) where
    M: embassy_sync::blocking_mutex::raw::RawMutex,
    D: embedded_services::sync::Lockable,
    PSU::Inner: psu::Psu,
    S: event::Sender<power_policy_interface::psu::event::EventData>,
    V: crate::wrapper::FwOfferValidator,
    <D as embedded_services::sync::Lockable>::Inner: crate::type_c::controller::Controller,
{
    task_closure(service, wrappers, cfu_client, |service: &Service<'_, PSU>| async {
        if let Err(e) = service.process_next_event().await {
            error!("Type-C service processing error: {:#?}", e);
        }
    })
    .await;
}
