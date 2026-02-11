use embassy_sync::{blocking_mutex::raw::RawMutex, mutex::Mutex};
use embedded_services::{error, event::Receiver, info, sync::Lockable};

use crate::psu::{Psu, event::RequestData};

use super::Service;

/// Runs the power policy task.
pub async fn task<const N: usize, M: RawMutex, D: Lockable + 'static, R: Receiver<RequestData> + 'static>(
    mut psu_events: crate::psu::event::EventReceivers<'static, N, D, R>,
    policy: &'static Mutex<M, Service<'static, D>>,
) -> !
where
    D::Inner: Psu,
{
    info!("Starting power policy task");
    loop {
        let event = psu_events.wait_event().await;

        if let Err(e) = policy.lock().await.process_psu_event(event).await {
            error!("Error processing request: {:?}", e);
        }
    }
}
