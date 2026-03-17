use embedded_services::{error, info, sync::Lockable};

use embedded_services::event::Receiver;
use power_policy_interface::psu::event::EventData;

use crate::service::registration::Registration;

use super::Service;

/// Runs the power policy task.
pub async fn task<
    'device,
    const PSU_COUNT: usize,
    S: Lockable<Inner = Service<'device, Reg>>,
    Reg: Registration<'device>,
    PsuReceiver: Receiver<EventData>,
>(
    mut psu_events: crate::psu::EventReceivers<'device, PSU_COUNT, Reg::Psu, PsuReceiver>,
    policy: &'device S,
) -> ! {
    info!("Starting power policy task");
    loop {
        let event = psu_events.wait_event().await;

        if let Err(e) = policy.lock().await.process_psu_event(event).await {
            error!("Error processing request: {:?}", e);
        }
    }
}
