#![no_std]
pub mod driver;
pub mod service;
pub mod task;
pub mod util;
pub mod wrapper;

use core::iter::Enumerate;

use type_c_interface::port::event::{
    PortEvent, PortEventBitfield, PortNotificationEventBitfield, PortStatusEventBitfield,
};

/// Struct to convert port events into a stream of events
#[derive(Clone)]
pub struct PortEventStreamer<Iter: Iterator<Item = PortEventBitfield>> {
    /// Iterator over pending event bitfields
    port_iter: Enumerate<Iter>,
    /// Notification to be streamed
    pending_notifications: Option<(usize, PortNotificationEventBitfield)>,
}

impl<Iter: Iterator<Item = PortEventBitfield>> PortEventStreamer<Iter> {
    /// Create new PortEventStreamer
    pub fn new(port_iter: Iter) -> Self {
        Self {
            port_iter: port_iter.enumerate(),
            pending_notifications: None,
        }
    }
}

impl<Iter: Iterator<Item = PortEventBitfield>> Iterator for PortEventStreamer<Iter> {
    type Item = (usize, PortEvent);

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            // Handle any pending notifications first
            if let Some((port_index, pending)) = &mut self.pending_notifications
                && let Some(port_event) = pending.next()
            {
                // Return a single notification
                return Some((*port_index, port_event));
            }

            // No pending notifications, fetch the next port event
            if let Some((port_index, event_bitfield)) = self.port_iter.next() {
                // Pending notifications for this port if there are any
                if event_bitfield.notification != PortNotificationEventBitfield::none() {
                    self.pending_notifications = Some((port_index, event_bitfield.notification));
                } else {
                    self.pending_notifications = None;
                }

                // Return a status changed event if there is one
                if event_bitfield.status != PortStatusEventBitfield::none() {
                    return Some((port_index, PortEvent::StatusChanged(event_bitfield.status)));
                }
            } else {
                // No more ports to process, we're done
                return None;
            }

            //Otherwise loop, to handle any remaining notifications
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    /// Utility function to create a PortStatusChanged event
    fn status_changed(plug_event: bool, power_contract: bool, sink_ready: bool) -> PortStatusEventBitfield {
        let mut status_changed = PortStatusEventBitfield::none();
        status_changed.set_plug_inserted_or_removed(plug_event);
        status_changed.set_new_power_contract_as_consumer(power_contract);
        status_changed.set_sink_ready(sink_ready);
        status_changed
    }

    /// Utility function to create a PortNotification event
    fn notification(alert: bool, discover_mode_completed: bool) -> PortNotificationEventBitfield {
        let mut notification = PortNotificationEventBitfield::none();
        notification.set_alert(alert);
        notification.set_discover_mode_completed(discover_mode_completed);
        notification
    }

    /// Test iterating over port status changed events
    #[test]
    fn test_port_status_changed() {
        let events = [
            status_changed(true, true, true).into(),
            status_changed(true, false, true).into(),
            status_changed(false, false, true).into(),
        ];
        let mut streamer = PortEventStreamer::new(events.iter().copied());

        assert_eq!(
            streamer.next(),
            Some((0, PortEvent::StatusChanged(status_changed(true, true, true))))
        );
        assert_eq!(
            streamer.next(),
            Some((1, PortEvent::StatusChanged(status_changed(true, false, true))))
        );
        assert_eq!(
            streamer.next(),
            Some((2, PortEvent::StatusChanged(status_changed(false, false, true))))
        );
        assert_eq!(streamer.next(), None);
    }

    /// Test iterating over port notifications
    #[test]
    fn test_port_notification() {
        let events = [notification(true, true).into()];
        let mut streamer = PortEventStreamer::new(events.iter().copied());

        assert_eq!(streamer.next(), Some((0, PortEvent::Alert)));
        assert_eq!(streamer.next(), Some((0, PortEvent::DiscoverModeCompleted)));
        assert_eq!(streamer.next(), None);
    }

    /// Test the final port with no pending notifications
    #[test]
    fn test_last_notifications() {
        let p0_event = status_changed(true, true, true).into();
        let events = [p0_event];
        let mut streamer = PortEventStreamer::new(events.iter().copied());

        assert_eq!(
            streamer.next(),
            Some((0, PortEvent::StatusChanged(status_changed(true, true, true))))
        );
        assert_eq!(streamer.next(), None);
    }

    /// Test iterating over both status and notification events
    #[test]
    fn test_port_event() {
        let p0_event = PortEventBitfield {
            status: status_changed(true, true, true),
            notification: notification(true, false),
        };
        let p1_event = PortEventBitfield {
            status: status_changed(false, true, false),
            notification: notification(false, true),
        };
        let events = [p0_event, p1_event];
        let mut streamer = PortEventStreamer::new(events.iter().copied());

        assert_eq!(
            streamer.next(),
            Some((0, PortEvent::StatusChanged(status_changed(true, true, true))))
        );
        assert_eq!(streamer.next(), Some((0, PortEvent::Alert)));
        assert_eq!(
            streamer.next(),
            Some((1, PortEvent::StatusChanged(status_changed(false, true, false))))
        );
        assert_eq!(streamer.next(), Some((1, PortEvent::DiscoverModeCompleted)));
        assert_eq!(streamer.next(), None);
    }

    /// Test no pending ports
    #[test]
    fn test_no_pending_ports() {
        let events: [PortEventBitfield; 0] = [];
        let mut streamer = PortEventStreamer::new(events.iter().copied());

        assert_eq!(streamer.next(), None);
    }

    /// Test a port with a pending event with no actual event
    #[test]
    fn test_empty_event() {
        let events = [PortEventBitfield::none()];
        let mut streamer = PortEventStreamer::new(events.iter().copied());

        assert_eq!(streamer.next(), None);
    }

    /// Test advancing to the next port when there are no events
    #[test]
    fn test_skip_no_pending() {
        let events = [PortEventBitfield::none(), status_changed(true, true, true).into()];
        let mut streamer = PortEventStreamer::new(events.iter().copied());

        assert_eq!(
            streamer.next(),
            Some((1, PortEvent::StatusChanged(status_changed(true, true, true))))
        );
        assert_eq!(streamer.next(), None);
    }
}
