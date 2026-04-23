//! This module contains event receiver types for the controller wrapper.
use core::array;
use core::future::pending;
use core::pin::pin;
use embassy_futures::select::{Either, Either4, select, select_slice, select4};
use embassy_time::{Instant, Ticker, Timer};
use embedded_services::{debug, trace};
use embedded_usb_pd::LocalPortId;

use crate::PortEventStreamer;
use crate::wrapper::DEFAULT_FW_UPDATE_TICK_INTERVAL_MS;
use crate::wrapper::cfu::FwUpdateState;
use crate::wrapper::message::{Event, EventCfu, LocalPortEvent, PowerPolicyCommand};
use crate::wrapper::proxy::PowerProxyReceiver;
use type_c_interface::port::event::{PortEvent, PortEventBitfield, PortStatusEventBitfield};

/// Trait used for receiving interrupt from the controller.
pub trait InterruptReceiver<const N: usize> {
    /// Wait for the next interrupt event.
    fn wait_interrupt(&mut self) -> impl Future<Output = [PortEventBitfield; N]>;
}

/// Struct to receive and stream port events from the controller.
pub struct PortEventReceiver<const N: usize, Receiver: InterruptReceiver<N>> {
    /// Receiver for the controller's interrupt events
    receiver: Receiver,
    /// Port event streaming state
    streaming_state: Option<PortEventStreamer<array::IntoIter<PortEventBitfield, N>>>,
}

impl<const N: usize, Receiver: InterruptReceiver<N>> PortEventReceiver<N, Receiver> {
    /// Create a new instance
    pub fn new(receiver: Receiver) -> Self {
        Self {
            receiver,
            streaming_state: None,
        }
    }

    /// Wait for the next port event
    pub async fn wait_next(&mut self) -> LocalPortEvent {
        loop {
            let streaming_state = if let Some(streaming_state) = &mut self.streaming_state {
                // Yield to ensure we don't monopolize the executor
                embassy_futures::yield_now().await;
                streaming_state
            } else {
                let events = self.receiver.wait_interrupt().await;
                self.streaming_state.insert(PortEventStreamer::new(events.into_iter()))
            };

            if let Some((port_index, event)) = streaming_state.next() {
                return LocalPortEvent {
                    port: LocalPortId(port_index as u8),
                    event,
                };
            } else {
                self.streaming_state = None;
            }
        }
    }
}

/// Struct to receive power policy messages.
pub struct ArrayPowerProxyEventReceiver<'device, const N: usize> {
    receivers: [PowerProxyReceiver<'device>; N],
}

impl<'device, const N: usize> ArrayPowerProxyEventReceiver<'device, N> {
    /// Create a new array power proxy event receiver
    pub fn new(receivers: [PowerProxyReceiver<'device>; N]) -> Self {
        Self { receivers }
    }

    /// Wait for the next power policy command
    pub async fn wait_next(&mut self) -> PowerPolicyCommand {
        let mut futures = heapless::Vec::<_, N>::new();
        for receiver in self.receivers.iter_mut() {
            // Size is fixed at compile time, so no chance of overflow
            let _ = futures.push(async { receiver.receive().await });
        }

        // DROP SAFETY: Select over drop safe futures
        let (request, local_id) = select_slice(pin!(futures.as_mut_slice())).await;
        trace!("Power command: device{} {:#?}", local_id, request);
        PowerPolicyCommand {
            port: LocalPortId(local_id as u8),
            request,
        }
    }

    /// Temporary function until the conversion to direct function calls is complete
    pub async fn send_response(
        &mut self,
        port: LocalPortId,
        response: power_policy_interface::psu::InternalResponseData,
    ) -> Result<(), ()> {
        self.receivers.get_mut(port.0 as usize).ok_or(())?.send(response).await;
        Ok(())
    }
}

/// Struct to receive CFU events.
pub struct CfuEventReceiver {
    /// FW update ticker used to check for timeouts and recovery attempts
    fw_update_ticker: Ticker,
    /// CFU device used for firmware updates
    cfu_device: &'static cfu_service::component::CfuDevice,
    pub fw_update_state: FwUpdateState,
}

impl CfuEventReceiver {
    /// Create a new CFU event receiver
    pub fn new(cfu_device: &'static cfu_service::component::CfuDevice) -> Self {
        Self {
            fw_update_ticker: Ticker::every(embassy_time::Duration::from_millis(DEFAULT_FW_UPDATE_TICK_INTERVAL_MS)),
            cfu_device,
            fw_update_state: FwUpdateState::Idle,
        }
    }

    /// Wait for the next CFU event
    pub async fn wait_next(&mut self) -> EventCfu {
        match self.fw_update_state {
            FwUpdateState::Idle => {
                // No FW update in progress, just wait for a command
                EventCfu::Request(self.cfu_device.wait_request().await)
            }
            FwUpdateState::InProgress(_) => {
                match select(self.cfu_device.wait_request(), self.fw_update_ticker.next()).await {
                    Either::First(command) => EventCfu::Request(command),
                    Either::Second(_) => {
                        debug!("FW update ticker ticked");
                        EventCfu::RecoveryTick
                    }
                }
            }
            FwUpdateState::Recovery => {
                // Recovery state, wait for the next attempt to recover the device
                self.fw_update_ticker.next().await;
                debug!("FW update ticker ticked");
                EventCfu::RecoveryTick
            }
        }
    }

    /// Reset the firmware update ticker
    pub fn reset_ticker(&mut self) {
        self.fw_update_ticker.reset();
    }
}

/// Struct to receive sink ready timeout events.
pub struct SinkReadyTimeoutEvent<const N: usize> {
    timeouts: [Option<Instant>; N],
}

impl<const N: usize> SinkReadyTimeoutEvent<N> {
    /// Create a new instance
    pub fn new() -> Self {
        Self { timeouts: [None; N] }
    }

    /// Set a timeout for a specific port
    pub fn set_timeout(&mut self, port: LocalPortId, new_timeout: Instant) {
        let index = port.0 as usize;
        if let Some(timeout) = self.timeouts.get_mut(index) {
            *timeout = Some(new_timeout);
        }
    }

    /// Clear the timeout for a specific port
    pub fn clear_timeout(&mut self, port: LocalPortId) {
        let index = port.0 as usize;
        if let Some(timeout) = self.timeouts.get_mut(index) {
            *timeout = None;
        }
    }

    pub fn get_timeout(&self, port: LocalPortId) -> Option<Instant> {
        let index = port.0 as usize;
        self.timeouts.get(index).copied().flatten()
    }

    /// Wait for a sink ready timeout and return the port that has timed out.
    ///
    /// DROP SAFETY: No state to restore
    pub async fn wait_next(&mut self) -> LocalPortId {
        let mut futures = heapless::Vec::<_, N>::new();
        for (i, timeout) in self.timeouts.iter().enumerate() {
            let timeout = *timeout;
            // Size is fixed at compile time, so no chance of overflow
            let _ = futures.push(async move {
                if let Some(timeout) = timeout {
                    Timer::at(timeout).await;
                    debug!("Port{}: Sink ready timeout reached", i);
                } else {
                    pending::<()>().await;
                }
            });
        }

        // DROP SAFETY: Select over drop safe futures
        let (_, port_index) = select_slice(pin!(futures.as_mut_slice())).await;
        if let Some(timeout) = self.timeouts.get_mut(port_index) {
            *timeout = None;
        }
        LocalPortId(port_index as u8)
    }
}

impl<const N: usize> Default for SinkReadyTimeoutEvent<N> {
    fn default() -> Self {
        Self::new()
    }
}

/// Struct used for containing controller event receivers.
pub struct ArrayPortEventReceivers<'device, const N: usize, PortInterrupts: InterruptReceiver<N>> {
    /// Port event receiver
    pub port_events: PortEventReceiver<N, PortInterrupts>,
    /// Power proxy event receiver
    pub power_proxies: ArrayPowerProxyEventReceiver<'device, N>,
    /// CFU event receiver
    pub cfu_event_receiver: CfuEventReceiver,
    /// Sink ready timeout event receiver
    pub sink_ready_timeout: SinkReadyTimeoutEvent<N>,
}

impl<'device, const N: usize, PortInterrupts: InterruptReceiver<N>>
    ArrayPortEventReceivers<'device, N, PortInterrupts>
{
    /// Create a new instance
    pub fn new(
        port_interrupts: PortInterrupts,
        power_proxies: [PowerProxyReceiver<'device>; N],
        cfu_device: &'static cfu_service::component::CfuDevice,
    ) -> Self {
        Self {
            port_events: PortEventReceiver::new(port_interrupts),
            power_proxies: ArrayPowerProxyEventReceiver::new(power_proxies),
            cfu_event_receiver: CfuEventReceiver::new(cfu_device),
            sink_ready_timeout: SinkReadyTimeoutEvent::new(),
        }
    }

    /// Wait for the next port event from any port.
    ///
    /// Returns the local port ID and the event bitfield.
    pub async fn wait_event(&mut self) -> Event {
        match select4(
            self.port_events.wait_next(),
            self.power_proxies.wait_next(),
            self.cfu_event_receiver.wait_next(),
            self.sink_ready_timeout.wait_next(),
        )
        .await
        {
            Either4::First(event) => Event::PortEvent(event),
            Either4::Second(command) => Event::PowerPolicyCommand(command),
            Either4::Third(cfu_event) => Event::CfuEvent(cfu_event),
            Either4::Fourth(port) => {
                let mut status_event = PortStatusEventBitfield::none();
                status_event.set_sink_ready(true);
                Event::PortEvent(LocalPortEvent {
                    port,
                    event: PortEvent::StatusChanged(status_event),
                })
            }
        }
    }
}
