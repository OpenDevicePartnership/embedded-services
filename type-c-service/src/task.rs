use embedded_services::{
    error,
    event::{self, Receiver},
    info,
    sync::Lockable,
};
use power_policy_interface::service::event::EventData as PowerPolicyEventData;

use crate::{
    service::{EventReceiver, Service},
    wrapper::ControllerWrapper,
};

/// Task to run the Type-C service, running the default event loop
pub async fn task<M, D, S, V, PowerReceiver: Receiver<PowerPolicyEventData>, const N: usize>(
    service: &'static impl Lockable<Inner = Service<'static>>,
    mut event_receiver: EventReceiver<'static, PowerReceiver>,
    wrappers: [&'static ControllerWrapper<'static, M, D, S, V>; N],
    cfu_client: &'static cfu_service::CfuClient,
) where
    M: embassy_sync::blocking_mutex::raw::RawMutex,
    D: embedded_services::sync::Lockable,
    S: event::Sender<power_policy_interface::psu::event::EventData>,
    V: crate::wrapper::FwOfferValidator,
    <D as embedded_services::sync::Lockable>::Inner: type_c_interface::port::Controller,
{
    info!("Starting type-c task");

    for controller_wrapper in wrappers {
        if controller_wrapper.register(cfu_client).is_err() {
            error!("Failed to register a controller");
            return;
        }
    }

    loop {
        let event = event_receiver.wait_next().await;
        if let Err(e) = service.lock().await.process_event(event).await {
            error!("Type-C service processing error: {:#?}", e);
        }
    }
}
