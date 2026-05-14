use core::pin::pin;

use crate::service::Event;
use embassy_futures::select::{Either, select, select_slice};
use embedded_services::{event::Receiver, sync::Lockable};
use power_policy_interface::service::event::EventData as PowerPolicyEventData;
use type_c_interface::{port::pd::Pd, service::event::PortEvent};

struct PowerPolicySubscriber<PowerReceiver: Receiver<PowerPolicyEventData>> {
    receiver: PowerReceiver,
}

impl<PowerReceiver: Receiver<PowerPolicyEventData>> PowerPolicySubscriber<PowerReceiver> {
    /// Wait for a power policy event
    async fn wait_next(&mut self) -> PowerPolicyEventData {
        self.receiver.wait_next().await
    }
}

pub struct ArrayPortReceivers<
    'port,
    const N: usize,
    Port: Lockable<Inner: Pd>,
    PortReceiver: Receiver<type_c_interface::service::event::PortEventData>,
> {
    ports: [&'port Port; N],
    port_receivers: [PortReceiver; N],
}

impl<
    'port,
    const N: usize,
    Port: Lockable<Inner: Pd>,
    PortReceiver: Receiver<type_c_interface::service::event::PortEventData>,
> ArrayPortReceivers<'port, N, Port, PortReceiver>
{
    /// Get the next pending PSU event
    pub async fn wait_next(&mut self) -> Event<'port, Port> {
        let ((event, port), _) = {
            let mut futures = heapless::Vec::<_, N>::new();
            for (receiver, psu) in self.port_receivers.iter_mut().zip(self.ports.iter()) {
                // Push will never fail since the number of receivers is the same as the capacity of the vector
                let _ = futures.push(async move { (receiver.wait_next().await, psu) });
            }
            select_slice(pin!(&mut futures)).await
        };

        Event::PortEvent(PortEvent { port: *port, event })
    }
}

/// Struct used to contain port event receivers and manage mapping from a receiver to its corresponding device.
pub struct ArrayEventReceiver<
    'a,
    const N: usize,
    Port: Lockable<Inner: Pd>,
    PortReceiver: Receiver<type_c_interface::service::event::PortEventData>,
    PowerReceiver: Receiver<PowerPolicyEventData>,
> {
    /// Power policy event subscriber
    power_policy_event_subscriber: PowerPolicySubscriber<PowerReceiver>,
    /// Port event receivers and corresponding ports
    port_receivers: ArrayPortReceivers<'a, N, Port, PortReceiver>,
}

impl<
    'port,
    const N: usize,
    Port: Lockable<Inner: Pd>,
    PortReceiver: Receiver<type_c_interface::service::event::PortEventData>,
    PowerReceiver: Receiver<PowerPolicyEventData>,
> ArrayEventReceiver<'port, N, Port, PortReceiver, PowerReceiver>
{
    /// Create a new instance
    pub fn new(
        ports: [&'port Port; N],
        port_receivers: [PortReceiver; N],
        power_policy_event_receiver: PowerReceiver,
    ) -> Self {
        Self {
            port_receivers: ArrayPortReceivers { ports, port_receivers },
            power_policy_event_subscriber: PowerPolicySubscriber {
                receiver: power_policy_event_receiver,
            },
        }
    }

    /// Wait for the next event, whether it's a port event or a power policy event
    pub async fn wait_next(&mut self) -> Event<'port, Port> {
        match select(
            self.port_receivers.wait_next(),
            self.power_policy_event_subscriber.wait_next(),
        )
        .await
        {
            Either::First(event) => event,
            Either::Second(event) => Event::PowerPolicy(event),
        }
    }
}
