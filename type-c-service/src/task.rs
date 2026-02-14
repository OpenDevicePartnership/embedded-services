use core::future::Future;
use embedded_services::{error, info, sync::Lockable};

use power_policy_service::{psu, service::context};

use crate::{service::Service, wrapper::ControllerWrapper};

/// Task to run the Type-C service, takes a closure to customize the event loop
pub async fn task_closure<
    'a,
    M,
    D,
    PSU: Lockable,
    V,
    Fut: Future<Output = ()>,
    F: Fn(&'a Service<'a, PSU>) -> Fut,
    const N: usize,
>(
    service: &'static Service<'a, PSU>,
    wrappers: [&'a ControllerWrapper<'a, M, D, V>; N],
    power_policy_context: &context::Context,
    cfu_client: &'a cfu_service::CfuClient,
    f: F,
) where
    M: embassy_sync::blocking_mutex::raw::RawMutex,
    D: Lockable,
    PSU::Inner: psu::Psu,
    V: crate::wrapper::FwOfferValidator,
    D::Inner: crate::type_c::controller::Controller,
{
    info!("Starting type-c task");

    if service.register_comms(power_policy_context).is_err() {
        error!("Failed to register type-c service endpoint");
        return;
    }

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
pub async fn task<'a, M, D, PSU: Lockable, V, const N: usize>(
    service: &'static Service<'a, PSU>,
    wrappers: [&'a ControllerWrapper<'a, M, D, V>; N],
    power_policy_context: &context::Context,
    cfu_client: &'a cfu_service::CfuClient,
) where
    M: embassy_sync::blocking_mutex::raw::RawMutex,
    D: embedded_services::sync::Lockable,
    PSU::Inner: psu::Psu,
    V: crate::wrapper::FwOfferValidator,
    <D as embedded_services::sync::Lockable>::Inner: crate::type_c::controller::Controller,
{
    task_closure(
        service,
        wrappers,
        power_policy_context,
        cfu_client,
        |service: &Service<'_, PSU>| async {
            if let Err(e) = service.process_next_event().await {
                error!("Type-C service processing error: {:#?}", e);
            }
        },
    )
    .await;
}
