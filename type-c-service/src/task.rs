use core::future::Future;
use embedded_services::{error, info};

use crate::{service::Service, wrapper::ControllerWrapper};

/// Task to run the Type-C service, takes a closure to customize the event loop
pub async fn task_closure<
    'a,
    M,
    C,
    V,
    Fut: Future<Output = ()>,
    F: Fn(&'a Service) -> Fut,
    const N: usize,
    const POLICY_CHANNEL_SIZE: usize,
>(
    service: &'static Service<'a>,
    wrappers: [&'a ControllerWrapper<'a, M, C, V, POLICY_CHANNEL_SIZE>; N],
    power_policy_context: &'a embedded_services::power::policy::policy::Context<POLICY_CHANNEL_SIZE>,
    f: F,
) where
    M: embassy_sync::blocking_mutex::raw::RawMutex,
    C: embedded_services::sync::Lockable,
    V: crate::wrapper::FwOfferValidator,
    <C as embedded_services::sync::Lockable>::Inner: embedded_services::type_c::controller::Controller,
{
    info!("Starting type-c task");

    if service.register_comms(power_policy_context).is_err() {
        error!("Failed to register type-c service endpoint");
        return;
    }

    for controller_wrapper in wrappers {
        if controller_wrapper
            .register(service.controllers(), power_policy_context)
            .await
            .is_err()
        {
            error!("Failed to register a controller");
            return;
        }
    }

    loop {
        f(service).await;
    }
}

/// Task to run the Type-C service, running the default event loop
pub async fn task<'a, M, C, V, const N: usize, const POLICY_CHANNEL_SIZE: usize>(
    service: &'static Service<'a>,
    wrappers: [&'a ControllerWrapper<'a, M, C, V, POLICY_CHANNEL_SIZE>; N],
    power_policy_context: &'a embedded_services::power::policy::policy::Context<POLICY_CHANNEL_SIZE>,
) where
    M: embassy_sync::blocking_mutex::raw::RawMutex,
    C: embedded_services::sync::Lockable,
    V: crate::wrapper::FwOfferValidator,
    <C as embedded_services::sync::Lockable>::Inner: embedded_services::type_c::controller::Controller,
{
    task_closure(service, wrappers, power_policy_context, |service: &Service| async {
        if let Err(e) = service.process_next_event().await {
            error!("Type-C service processing error: {:#?}", e);
        }
    })
    .await;
}
