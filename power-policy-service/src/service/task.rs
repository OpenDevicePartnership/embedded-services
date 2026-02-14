use embassy_sync::{blocking_mutex::raw::RawMutex, mutex::Mutex};
use embedded_services::{error, event::Receiver, info, sync::Lockable};

use crate::psu::{Psu, event::EventData};

use super::Service;

/// Runs the power policy task.
pub async fn task<const PSU_COUNT: usize, M: RawMutex, PSU: Lockable, R: Receiver<EventData>>(
    mut psu_events: crate::psu::event::EventReceivers<'static, PSU_COUNT, PSU, R>,
    policy: &'static Mutex<M, Service<'static, PSU>>,
) -> !
where
    PSU::Inner: Psu,
{
    info!("Starting power policy task");
    loop {
        let event = psu_events.wait_event().await;

        if let Err(e) = policy.lock().await.process_psu_event(event).await {
            error!("Error processing request: {:?}", e);
        }
    }
}
