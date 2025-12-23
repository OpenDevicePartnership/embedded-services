#![no_std]
pub mod driver;
pub mod service;
pub mod task;
pub mod wrapper;

use core::future::Future;

use embedded_services::type_c::event::{
    PortEvent, PortNotification, PortNotificationSingle, PortPendingIter, PortStatusChanged,
};
pub use task::task;

/// Enum to contain all port event variants
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum PortEventVariant {
    /// Port status change events
    StatusChanged(PortStatusChanged),
    /// Port notification events
    Notification(PortNotificationSingle),
}

/// Struct to convert port events into a stream of events
#[derive(Clone, Copy)]
pub struct PortEventStreamer {
    /// Current port index being processed
    port_index: Option<usize>,
    /// Iterator over pending ports
    pending_iter: PortPendingIter,
    /// Notification to be streamed
    pending_notifications: Option<PortNotification>,
}

impl PortEventStreamer {
    /// Create new PortEventStreamer
    ///
    /// Returns none if there are no pending ports to process.
    pub fn new(pending_iter: PortPendingIter) -> Self {
        Self {
            port_index: None,
            pending_iter,
            pending_notifications: None,
        }
    }
}

impl PortEventStreamer {
    /// Get the next port event, calls the closure if it needs to get pending events for the current port.
    pub async fn next<E, Fut: Future<Output = Result<PortEvent, E>>, F: FnMut(usize) -> Fut>(
        &mut self,
        mut f: F,
    ) -> Result<Option<(usize, PortEventVariant)>, E> {
        loop {
            let port_index = if let Some(index) = self.port_index {
                index
            } else if let Some(next_port) = self.pending_iter.next() {
                // First time this function is called, get our starting port index
                self.port_index = Some(next_port);
                next_port
            } else {
                // No pending ports to process
                return Ok(None);
            };

            let mut advance_port = false;
            let mut ret = None;

            if let Some(mut pending) = self.pending_notifications {
                if let Some(port_event) = pending.next() {
                    // Return a single notification
                    self.pending_notifications = Some(pending);
                    ret = Some((port_index, PortEventVariant::Notification(port_event)));
                } else {
                    // Done with pending notifications, continue to the next port
                    advance_port = true;
                    self.pending_notifications = None;
                }
            } else {
                // Haven't read port events yet
                let event = f(port_index).await?;

                if event.notification != PortNotification::none() {
                    // Have pending notifications to stream as events, store those for the next loop/call to this function
                    self.pending_notifications = Some(event.notification);
                } else {
                    // No pending notifications, we can advance to the next port
                    advance_port = true;
                    self.pending_notifications = None;
                }

                if event.status != PortStatusChanged::none() {
                    // Return the port status changed event first if there is one
                    ret = Some((port_index, PortEventVariant::StatusChanged(event.status)));
                }
            }

            if advance_port {
                if let Some(next_port) = self.pending_iter.next() {
                    // Move to the next port
                    self.port_index = Some(next_port);
                } else if ret.is_none() {
                    // Don't have any more ports to process
                    // And we didn't have any events to return, we're done
                    return Ok(None);
                } else {
                    // This is the last port, but we have an event to return
                    // We'll have to return none on the next call, achieve this by setting port_index to None
                    // The next call will call next() on the pending port iterator which will return None
                    self.port_index = None;
                }
            }

            // Return the event if we have one, otherwise loop to get the next event
            if ret.is_some() {
                return Ok(ret);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use core::sync::atomic::AtomicBool;

    use embedded_services::type_c::event::PortPending;

    use super::*;

    /// Utitily function to create a PortStatusChanged event
    fn status_changed(plug_event: bool, power_contract: bool, sink_ready: bool) -> PortStatusChanged {
        let mut status_changed = PortStatusChanged::none();
        status_changed.set_plug_inserted_or_removed(plug_event);
        status_changed.set_new_power_contract_as_consumer(power_contract);
        status_changed.set_sink_ready(sink_ready);
        status_changed
    }

    /// Utility function to create a PortNotification event
    fn notification(alert: bool, discover_mode_completed: bool) -> PortNotification {
        let mut notification = PortNotification::none();
        notification.set_alert(alert);
        notification.set_discover_mode_completed(discover_mode_completed);
        notification
    }

    /// Test iterating over port status changed events
    #[tokio::test]
    async fn test_port_status_changed() {
        let mut pending_ports = PortPending::none();
        pending_ports.pend_port(0).unwrap();
        pending_ports.pend_port(2).unwrap();
        pending_ports.pend_port(3).unwrap();

        let mut streamer = PortEventStreamer::new(pending_ports.into_iter());

        let event = streamer
            .next::<(), _, _>(async |_| Ok(status_changed(true, true, true).into()))
            .await;
        assert_eq!(
            event,
            Ok(Some((
                0,
                PortEventVariant::StatusChanged(status_changed(true, true, true))
            )))
        );

        let event = streamer
            .next::<(), _, _>(async |_| Ok(status_changed(true, false, true).into()))
            .await;
        assert_eq!(
            event,
            Ok(Some((
                2,
                PortEventVariant::StatusChanged(status_changed(true, false, true))
            )))
        );

        let event = streamer
            .next::<(), _, _>(async |_| Ok(status_changed(false, false, true).into()))
            .await;
        assert_eq!(
            event,
            Ok(Some((
                3,
                PortEventVariant::StatusChanged(status_changed(false, false, true))
            )))
        );

        let event = streamer
            .next::<(), _, _>(async |_| Ok(status_changed(false, false, true).into()))
            .await;
        assert_eq!(event, Ok(None));
    }

    /// Test iterating over port notifications
    #[tokio::test]
    async fn test_port_notification() {
        let mut pending_ports = PortPending::none();
        pending_ports.pend_port(0).unwrap();

        let mut streamer = PortEventStreamer::new(pending_ports.into_iter());
        let event = streamer
            .next::<(), _, _>(async |_| Ok(notification(true, true).into()))
            .await;
        assert_eq!(
            event,
            Ok(Some((0, PortEventVariant::Notification(PortNotificationSingle::Alert))))
        );

        let event = streamer
            .next::<(), _, _>(async |_| Ok(notification(true, true).into()))
            .await;
        assert_eq!(
            event,
            Ok(Some((
                0,
                PortEventVariant::Notification(PortNotificationSingle::DiscoverModeCompleted)
            )))
        );

        let event = streamer
            .next::<(), _, _>(async |_| Ok(notification(true, true).into()))
            .await;
        assert_eq!(event, Ok(None));
    }

    /// Test the the final port with no pending notifications
    #[tokio::test]
    async fn test_last_notifications() {
        let mut pending_ports = PortPending::none();
        pending_ports.pend_port(0).unwrap();

        let mut streamer = PortEventStreamer::new(pending_ports.into_iter());

        // Test p0 events
        let p0_event = status_changed(true, true, true).into();
        let event = streamer.next::<(), _, _>(async |_| Ok(p0_event)).await;
        assert_eq!(
            event,
            Ok(Some((
                0,
                PortEventVariant::StatusChanged(status_changed(true, true, true))
            )))
        );

        let event = streamer.next::<(), _, _>(async |_| Ok(p0_event)).await;
        assert_eq!(event, Ok(None));
    }

    /// Test iterating over both status and notification events
    #[tokio::test]
    async fn test_port_event() {
        let mut pending_ports = PortPending::none();
        pending_ports.pend_port(0).unwrap();
        pending_ports.pend_port(6).unwrap();

        let mut streamer = PortEventStreamer::new(pending_ports.into_iter());

        // Test p0 events
        let p0_event = PortEvent {
            status: status_changed(true, true, true),
            notification: notification(true, false),
        };

        let event = streamer.next::<(), _, _>(async |_| Ok(p0_event)).await;
        assert_eq!(
            event,
            Ok(Some((
                0,
                PortEventVariant::StatusChanged(status_changed(true, true, true))
            )))
        );

        let event = streamer.next::<(), _, _>(async |_| Ok(p0_event)).await;
        assert_eq!(
            event,
            Ok(Some((0, PortEventVariant::Notification(PortNotificationSingle::Alert))))
        );

        // Test p6 events
        let p6_event = PortEvent {
            status: status_changed(false, true, false),
            notification: notification(false, true),
        };

        let event = streamer.next::<(), _, _>(async |_| Ok(p6_event)).await;
        assert_eq!(
            event,
            Ok(Some((
                6,
                PortEventVariant::StatusChanged(status_changed(false, true, false))
            )))
        );

        let event = streamer.next::<(), _, _>(async |_| Ok(p6_event)).await;
        assert_eq!(
            event,
            Ok(Some((
                6,
                PortEventVariant::Notification(PortNotificationSingle::DiscoverModeCompleted)
            )))
        );

        let event = streamer.next::<(), _, _>(async |_| Ok(p6_event)).await;
        assert_eq!(event, Ok(None));
    }

    /// Test no pending ports
    #[tokio::test]
    async fn test_no_pending_ports() {
        let pending_ports = PortPending::none();
        let mut streamer = PortEventStreamer::new(pending_ports.into_iter());
        let event = streamer
            .next::<(), _, _>(async |_| Ok(status_changed(true, true, true).into()))
            .await;
        assert_eq!(event, Ok(None));
    }

    /// Test a port with a pending event with no actual event
    #[tokio::test]
    async fn test_empty_event() {
        let mut pending_ports = PortPending::none();
        pending_ports.pend_port(0).unwrap();

        let mut streamer = PortEventStreamer::new(pending_ports.into_iter());
        let event = streamer.next::<(), _, _>(async |_| Ok(PortEvent::none())).await;
        assert_eq!(event, Ok(None));
    }

    /// Test advancing to the next port when there are no events
    #[tokio::test]
    async fn test_skip_no_pending() {
        let mut pending_ports = PortPending::none();
        pending_ports.pend_port(0).unwrap();
        pending_ports.pend_port(1).unwrap();

        let mut streamer = PortEventStreamer::new(pending_ports.into_iter());
        let event = streamer
            .next::<(), _, _>(async |_| {
                static HAVE_EVENTS: AtomicBool = AtomicBool::new(false);
                let have_events = HAVE_EVENTS.load(core::sync::atomic::Ordering::Relaxed);
                let event = Ok(status_changed(have_events, have_events, have_events).into());
                HAVE_EVENTS.store(true, core::sync::atomic::Ordering::Relaxed);
                event
            })
            .await;
        assert_eq!(
            event,
            Ok(Some((
                1,
                PortEventVariant::StatusChanged(status_changed(true, true, true))
            )))
        );

        let event = streamer
            .next::<(), _, _>(async |_| Ok(status_changed(false, false, false).into()))
            .await;
        assert_eq!(event, Ok(None));
    }
}
