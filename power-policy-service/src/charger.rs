use core::pin::pin;

use embassy_futures::select::select_slice;
use embedded_services::event::Receiver;
use embedded_services::sync::Lockable;
use power_policy_interface::charger::Charger;
use power_policy_interface::charger::event::{Event, EventData};

/// Struct used to contain charger event receivers and manage mapping from a receiver to its corresponding device.
pub struct ChargerEventReceivers<'a, const N: usize, CHARGER: Lockable, R: Receiver<EventData>>
where
    CHARGER::Inner: Charger,
{
    pub charger_devices: [&'a CHARGER; N],
    pub receivers: [R; N],
}

impl<'a, const N: usize, CHARGER: Lockable, R: Receiver<EventData>> ChargerEventReceivers<'a, N, CHARGER, R>
where
    CHARGER::Inner: Charger,
{
    /// Create a new instance
    pub fn new(charger_devices: [&'a CHARGER; N], receivers: [R; N]) -> Self {
        Self {
            charger_devices,
            receivers,
        }
    }

    /// Get the next pending charger event
    pub async fn wait_event(&mut self) -> Event<'a, CHARGER> {
        let ((event, charger), _) = {
            let mut futures = heapless::Vec::<_, N>::new();
            for (receiver, psu) in self.receivers.iter_mut().zip(self.charger_devices.iter()) {
                // Push will never fail since the number of receivers is the same as the capacity of the vector
                let _ = futures.push(async move { (receiver.wait_next().await, psu) });
            }
            select_slice(pin!(&mut futures)).await
        };

        Event { charger, event }
    }
}
