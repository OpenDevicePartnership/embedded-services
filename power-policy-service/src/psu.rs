use core::pin::pin;

use embassy_futures::select::select_slice;
use embedded_services::event::Receiver;
use embedded_services::sync::Lockable;
use power_policy_interface::psu::Psu;
use power_policy_interface::psu::event::{Event, EventData};

/// Struct used to contain PSU event receivers and manage mapping from a receiver to its corresponding device.
pub struct ArrayEventReceivers<'a, const N: usize, PSU: Lockable, R: Receiver<EventData>>
where
    PSU::Inner: Psu,
{
    pub psu_devices: [&'a PSU; N],
    pub receivers: [R; N],
}

impl<'a, const N: usize, PSU: Lockable, R: Receiver<EventData>> ArrayEventReceivers<'a, N, PSU, R>
where
    PSU::Inner: Psu,
{
    /// Create a new instance
    pub fn new(psu_devices: [&'a PSU; N], receivers: [R; N]) -> Self {
        Self { psu_devices, receivers }
    }

    /// Get the next pending PSU event
    pub async fn wait_event(&mut self) -> Event<'a, PSU> {
        let ((event, psu), _) = {
            let mut futures = heapless::Vec::<_, N>::new();
            for (receiver, psu) in self.receivers.iter_mut().zip(self.psu_devices.iter()) {
                // Push will never fail since the number of receivers is the same as the capacity of the vector
                let _ = futures.push(async move { (receiver.wait_next().await, psu) });
            }
            select_slice(pin!(&mut futures)).await
        };

        Event { psu, event }
    }
}
