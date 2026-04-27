use embedded_services::{error, info, sync::Lockable};

use embedded_services::event::Receiver;
use power_policy_interface::charger;
use power_policy_interface::psu::event::EventData;

use crate::service::registration::Registration;

use super::Service;

/// Runs the power policy PSU task.
pub async fn psu_task<
    'device,
    const PSU_COUNT: usize,
    S: Lockable<Inner = Service<'device, Reg>>,
    Reg: Registration<'device>,
    PsuReceiver: Receiver<EventData>,
>(
    mut psu_events: crate::psu::PsuEventReceivers<'device, PSU_COUNT, Reg::Psu, PsuReceiver>,
    policy: &'device S,
) -> ! {
    info!("Starting power policy PSU task");
    loop {
        let event = psu_events.wait_event().await;

        if let Err(e) = policy.lock().await.process_psu_event(event).await {
            error!("Error processing request: {:?}", e);
        }
    }
}

/// Runs the power policy charger task.
pub async fn charger_task<
    'device,
    const CHARGER_COUNT: usize,
    S: Lockable<Inner = Service<'device, Reg>>,
    Reg: Registration<'device>,
    ChargerReceiver: Receiver<charger::EventData>,
>(
    mut charger_events: crate::charger::ChargerEventReceivers<'device, CHARGER_COUNT, Reg::Charger, ChargerReceiver>,
    policy: &'device S,
) -> ! {
    info!("Starting power policy charger task");
    loop {
        let event = charger_events.wait_event().await;

        if let Err(e) = policy.lock().await.process_charger_event(event).await {
            error!("Error processing request: {:?}", e);
        }
    }
}

/// Runs the power policy unified task.
pub async fn task<
    'device,
    const PSU_COUNT: usize,
    const CHARGER_COUNT: usize,
    S: Lockable<Inner = Service<'device, Reg>>,
    Reg: Registration<'device>,
    PsuReceiver: Receiver<EventData>,
    ChargerReceiver: Receiver<charger::EventData>,
>(
    mut psu_events: crate::psu::PsuEventReceivers<'device, PSU_COUNT, Reg::Psu, PsuReceiver>,
    mut charger_events: crate::charger::ChargerEventReceivers<'device, CHARGER_COUNT, Reg::Charger, ChargerReceiver>,
    policy: &'device S,
) -> ! {
    info!("Starting power policy task");
    loop {
        match embassy_futures::select::select(psu_events.wait_event(), charger_events.wait_event()).await {
            embassy_futures::select::Either::First(psu_event) => {
                if let Err(e) = policy.lock().await.process_psu_event(psu_event).await {
                    error!("Error processing PSU request: {:?}", e);
                }
            }
            embassy_futures::select::Either::Second(charger_event) => {
                if let Err(e) = policy.lock().await.process_charger_event(charger_event).await {
                    error!("Error processing charger request: {:?}", e);
                }
            }
        }
    }
}
