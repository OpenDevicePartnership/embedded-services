//! This module contains event receiver types for the controller wrapper.
use core::array;
use core::future::pending;
use embassy_futures::select::{Either, select};
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

/// Struct to receive and stream port events from the controller.
pub struct PortEventReceiver<R: Receiver<PortEventBitfield>, LoopbackReceiver: Receiver<Loopback>> {
    /// Receiver for the controller's interrupt events
    receiver: R,
    /// Port event streaming state
    streaming_state: Option<PortEventStreamer<array::IntoIter<PortEventBitfield, 1>>>,
    /// Loopback receiver for software-generated events
    loopback_receiver: LoopbackReceiver,
}

impl<R: Receiver<PortEventBitfield>, LoopbackReceiver: Receiver<Loopback>> PortEventReceiver<R, LoopbackReceiver> {
    /// Create a new instance
    pub fn new(receiver: R, loopback_receiver: LoopbackReceiver) -> Self {
        Self {
            receiver,
            streaming_state: None,
            loopback_receiver,
        }
    }

    /// Wait for the next port event
    pub async fn wait_next(&mut self) -> type_c_interface::port::event::PortEvent {
        loop {
            let streaming_state = if let Some(streaming_state) = &mut self.streaming_state {
                // Yield to ensure we don't monopolize the executor
                embassy_futures::yield_now().await;
                streaming_state
            } else {
                let (Either::First(Loopback::PortEvent(events)) | Either::Second(events)) =
                    select(self.loopback_receiver.wait_next(), self.receiver.wait_next()).await;
                self.streaming_state
                    .insert(PortEventStreamer::new([events].into_iter()))
            };

            if let Some((_, event)) = streaming_state.next() {
                return event;
            } else {
                self.streaming_state = None;
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
    port_event_receiver: PortEventReceiver<InterruptReceiver, LoopbackReceiver>,
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
            port_event_receiver: PortEventReceiver::new(port_event_receiver, loopback_receiver),
        }
    }

    /// Wait for the next port event from any port.
    ///
    /// Returns the local port ID and the event bitfield.
    pub async fn wait_event(&mut self) -> Event {
        let timeout = self.shared_state.lock().await.sink_ready_timeout;
        match select(self.port_event_receiver.wait_next(), async move {
            if let Some(timeout) = timeout {
                Timer::at(timeout).await;
            } else {
                pending::<()>().await;
            }
        })
        .await
        {
            Either::First(event) => Event::PortEvent(event),
            Either::Second(_) => {
                let mut status_event = PortStatusEventBitfield::none();
                status_event.set_sink_ready(true);
                self.shared_state.lock().await.sink_ready_timeout = None;
                Event::PortEvent(PortEvent::StatusChanged(status_event))
            }
        }
    }
}
