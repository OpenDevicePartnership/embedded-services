//! This module contains event receiver types for the controller wrapper.
use core::array;
use core::future::pending;
use embassy_futures::select::{Either3, select3};
use embassy_time::Timer;
use embedded_services::error;
use embedded_services::event::{NonBlockingSender, Receiver};
use embedded_services::sync::Lockable;

use crate::PortEventStreamer;
use crate::controller::event::{Event, Loopback};
use crate::controller::state::SharedState;
use type_c_interface::port::event::{PortEvent, PortEventBitfield, PortStatusEventBitfield};

/// Trait used for receiving interrupt from the controller.
pub trait InterruptReceiver<const N: usize> {
    /// Wait for the next interrupt event.
    fn wait_interrupt(&mut self) -> impl Future<Output = [PortEventBitfield; N]>;
}

/// Struct to send received interrupts to their corresponding port receivers
pub struct PortEventSplitter<const N: usize, S: NonBlockingSender<PortEventBitfield>> {
    /// Senders to forward port events to their corresponding port receivers
    sender: [S; N],
}

impl<const N: usize, S: NonBlockingSender<PortEventBitfield>> PortEventSplitter<N, S> {
    /// Create a new instance
    pub fn new(sender: [S; N]) -> Self {
        Self { sender }
    }

    /// Wait for the next interrupt event and forward it to the corresponding port receiver.
    pub async fn process_interrupts(&mut self, interrupts: [PortEventBitfield; N]) {
        for (interrupt, sender) in interrupts.into_iter().zip(self.sender.iter_mut()) {
            if interrupt != PortEventBitfield::none() && sender.try_send(interrupt).is_none() {
                error!("Failed to send port event");
            }
        }
    }
}

/// Struct used for containing controller event receivers.
pub struct EventReceiver<
    'a,
    State: Lockable<Inner = SharedState>,
    InterruptReceiver: Receiver<PortEventBitfield>,
    LoopbackReceiver: Receiver<Loopback>,
> {
    /// Port event receiver
    port_event_receiver: InterruptReceiver,
    /// Port event streaming state
    streaming_state: Option<PortEventStreamer<array::IntoIter<PortEventBitfield, 1>>>,
    /// Loopback event receiver
    loopback_receiver: LoopbackReceiver,
    /// Shared state
    shared_state: &'a State,
}

impl<
    'a,
    State: Lockable<Inner = SharedState>,
    InterruptReceiver: Receiver<PortEventBitfield>,
    LoopbackReceiver: Receiver<Loopback>,
> EventReceiver<'a, State, InterruptReceiver, LoopbackReceiver>
{
    /// Create a new instance
    pub fn new(
        shared_state: &'a State,
        port_event_receiver: InterruptReceiver,
        loopback_receiver: LoopbackReceiver,
    ) -> Self {
        Self {
            shared_state,
            port_event_receiver,
            streaming_state: None,
            loopback_receiver,
        }
    }

    /// Wait for the next port event from a single port.
    pub async fn wait_event(&mut self) -> Event {
        loop {
            if let Some(streaming_state) = &mut self.streaming_state {
                // If we have a streaming state, prioritize processing it before waiting for new events. This
                // ensures that any pending events stay buffered in the receiver.

                // Yield to ensure we don't monopolize the executor
                embassy_futures::yield_now().await;

                if let Some((_, event)) = streaming_state.next() {
                    return Event::PortEvent(event);
                }

                // Done streaming, clear the state and continue to wait for new events.
                self.streaming_state = None;
            } else {
                let timeout = self.shared_state.lock().await.sink_ready_deadline;
                match select3(
                    self.port_event_receiver.wait_next(),
                    async move {
                        if let Some(timeout) = timeout {
                            Timer::at(timeout).await;
                        } else {
                            pending::<()>().await;
                        }
                    },
                    self.loopback_receiver.wait_next(),
                )
                .await
                {
                    Either3::First(events) => {
                        self.streaming_state = Some(PortEventStreamer::new([events].into_iter()));
                    }
                    Either3::Second(_) => {
                        let mut status_event = PortStatusEventBitfield::none();
                        status_event.set_sink_ready(true);
                        self.shared_state.lock().await.sink_ready_deadline = None;
                        return Event::PortEvent(PortEvent::StatusChanged(status_event));
                    }
                    Either3::Third(event) => match event {
                        Loopback::PortEvent(events) => {
                            self.streaming_state = Some(PortEventStreamer::new([events].into_iter()));
                            // Continue, the next iteration will handle streaming the port events.
                        }
                        Loopback::SinkReadyDeadlineInvalidated => {
                            // Continue, the next iteration will wait for the update sink ready deadline.
                        }
                    },
                }
            }
        }
    }
}
