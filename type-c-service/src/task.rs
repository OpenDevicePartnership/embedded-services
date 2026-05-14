use embedded_services::{error, event::Receiver, info, sync::Lockable};
use power_policy_interface::service::event::EventData as PowerPolicyEventData;
use type_c_interface::port::pd::Pd;

use crate::service::{Service, event_receiver::ArrayEventReceiver, registration::Registration};

/// Task to run the Type-C service, running the default event loop
pub async fn task<
    const N: usize,
    Port: Lockable<Inner: Pd>,
    PortReceiver: Receiver<type_c_interface::service::event::PortEventData>,
    PowerReceiver: Receiver<PowerPolicyEventData>,
>(
    service: &'static impl Lockable<Inner = Service<'static, impl Registration<'static, Port = Port>>>,
    mut event_receiver: ArrayEventReceiver<'static, N, Port, PortReceiver, PowerReceiver>,
) {
    info!("Starting type-c task");

    loop {
        let event = event_receiver.wait_next().await;
        if let Err(e) = service.lock().await.process_event(event).await {
            error!("Type-C service processing error: {:#?}", e);
        }
    }
}
