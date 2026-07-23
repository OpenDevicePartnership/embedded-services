//! Mock HID mouse device used by the `mock_i2c_mouse` example
//!
//! This demonstrates a zero-copy input path: input reports live in a caller-provided ring buffer
//! and are written/read *in place, which saves a memcpy at the cost of some complexity.

use defmt::info;
use embassy_sync::zerocopy_channel;
use embedded_services::GlobalRawMutex;
use embedded_services::relay::hid::*;
use zerocopy::IntoBytes;

// This is adapted from the example mouse HID descriptor packaged with the DT.exe tool / https://learn.microsoft.com/en-us/windows-hardware/design/component-guidelines/mouse-collection-report-descriptor
const REPORTID_MOUSE: u8 = 1;

#[rustfmt::skip]
const MOUSE_HID_REPORT_DESCRIPTOR: &[u8] = &[
    0x05, 0x01, // Usage Page (Generic Desktop Ctrls)
    0x09, 0x02, // Usage (Mouse)
    0xA1, 0x01, // Collection (Application)
    0x85, REPORTID_MOUSE, //   REPORT_ID (Touch pad) **** THIS IS ADAPTED FROM SAMPLE TOUCHPAD DESCRIPTOR, THE MOUSE EXAMPLE OMITTED IT BECAUSSE IT ONLY HAD 1 REPORT
    0x09, 0x01, //   Usage (Pointer)
    0xA1, 0x00, //   Collection (Physical)
    0x05, 0x09, //     Usage Page (Button)
    0x19, 0x01, //     Usage Minimum (0x01)
    0x29, 0x03, //     Usage Maximum (0x03)
    0x15, 0x00, //     Logical Minimum (0)
    0x25, 0x01, //     Logical Maximum (1)
    0x95, 0x03, //     Report Count (3)
    0x75, 0x01, //     Report Size (1)
    0x81, 0x02, //     Input (Data,Var,Abs,No Wrap,Linear,Preferred State,No Null Position)
    0x95, 0x01, //     Report Count (1)
    0x75, 0x05, //     Report Size (5)
    0x81, 0x03, //     Input (Const,Var,Abs,No Wrap,Linear,Preferred State,No Null Position)
    0x05, 0x01, //     Usage Page (Generic Desktop Ctrls)
    0x09, 0x30, //     Usage (X)
    0x09, 0x31, //     Usage (Y)
    0x15, 0x81, //     Logical Minimum (-127)
    0x25, 0x7F, //     Logical Maximum (127)
    0x75, 0x08, //     Report Size (8)
    0x95, 0x02, //     Report Count (2)
    0x81, 0x06, //     Input (Data,Var,Rel,No Wrap,Linear,Preferred State,No Null Position)
    0xC0, //   End Collection
    0xC0, // End Collection
];

const MOUSE_BUTTON_1: u8 = 0x01;
#[allow(dead_code)]
const MOUSE_BUTTON_2: u8 = 0x02;
#[allow(dead_code)]
const MOUSE_BUTTON_3: u8 = 0x04;

#[repr(C)]
#[derive(Debug, Default, defmt::Format, zerocopy::FromBytes, zerocopy::IntoBytes, zerocopy::Immutable)]
struct MouseReport {
    buttons: u8, // 3 bits used for buttons, 5 bits padding
    x: i8,
    y: i8,
}

// Number of in-flight input reports the zero-copy channel can hold. Depth >= 2 lets the producer
// stage the next report while the consumer is still handing the current one to the host.
const MOUSE_CHANNEL_DEPTH: usize = 4;

type MouseChannel<'d> = zerocopy_channel::Channel<'d, GlobalRawMutex, MouseReport>;
type MouseSender<'d> = zerocopy_channel::Sender<'d, GlobalRawMutex, MouseReport>;
type MouseReceiver<'d> = zerocopy_channel::Receiver<'d, GlobalRawMutex, MouseReport>;

/// Memory resources for the mock mouse service.
///
/// Following the pattern used by services like `time_alarm_service` and `hidi2c_target_service`, the
/// caller allocates a single `Resources` object (for example in a `StaticCell`) and hands a mutable
/// borrow to [`MockMouseService::new`] at construction time, instead of scattering `static`
/// singletons around. The struct is generic over the lifetime `'d` of that borrow rather than being
/// pinned to `'static`.
pub struct MockMouseResources<'d> {
    /// Backing storage for the zero-copy channel's ring buffer.
    buf: [MouseReport; MOUSE_CHANNEL_DEPTH],
    /// The channel itself, created in [`MockMouseService::new`] from a borrow of `buf`.
    channel: Option<MouseChannel<'d>>,
}

impl Default for MockMouseResources<'_> {
    fn default() -> Self {
        Self {
            buf: core::array::from_fn(|_| MouseReport::default()),
            channel: None,
        }
    }
}

/// Consumer side of the mock mouse. Owns the channel `Receiver` and hands the host borrows that
/// point directly into the ring buffer — no intermediate copy of the report payload.
pub struct MockMouseService<'hw> {
    receiver: MouseReceiver<'hw>,
}

impl<'hw> MockMouseService<'hw> {
    /// Wire up the zero-copy channel inside `resources` and split it into the service (which the
    /// relay borrows) and a runner (used to inject mouse events).
    pub fn new(resources: &'hw mut MockMouseResources<'hw>) -> (Self, MockMouseRunner<'hw>) {
        // Destructure so the buffer and channel fields are borrowed disjointly: the channel borrows
        // the buffer in place, and both live as long as `resources`.
        let MockMouseResources { buf, channel } = resources;
        let channel = channel.insert(MouseChannel::new(buf));
        let (sender, receiver) = channel.split();
        (Self { receiver }, MockMouseRunner { sender })
    }

    // Event receiver for the service. If you truly need this sort of zero-copy approach all the way from the service through the relay adapter to the tranport,
    // you may have to make some layout concessions in the service.  This implies that it's probably not possible to support zerocopy for a given message type
    // across multiple tranports that have different layout requirements, and some knowledge of which layout to use may need to leak into the service for
    // services that truly need zero-copy messages.  This is probably not going to be common, though - in the case of a mouse, for example, the biggest report is
    // 3 bytes, so the copy is trivial and the service in practice would just use a regular channel rather than a zerocopy channel.
    fn receiver(&mut self) -> &mut MouseReceiver<'hw> {
        &mut self.receiver
    }
}

/// In a production case, this would implement the Runner trait and be spawned as a task, but for the mock we just expose methods to inject mouse events
pub struct MockMouseRunner<'d> {
    sender: MouseSender<'d>,
}

impl MockMouseRunner<'_> {
    /// Acquire the next free slot in the ring buffer and populate it in place, then publish it.
    async fn send(&mut self, report: MouseReport) {
        // `send()` waits for a free slot and yields `&mut MouseReport` pointing straight into the
        // channel's ring buffer. Writing through it avoids copying the payload into the channel.
        let slot = self.sender.send().await;
        *slot = report;
        // Publish the slot to the consumer. Nothing is copied here either.
        self.sender.send_done();
    }

    pub async fn send_click(&mut self) {
        // Mouse down
        self.send(MouseReport {
            buttons: MOUSE_BUTTON_1,
            x: 0,
            y: 0,
        })
        .await;

        embassy_time::Timer::after(embassy_time::Duration::from_millis(15)).await;

        // Mouse up
        self.send(MouseReport { buttons: 0, x: 0, y: 0 }).await;
    }

    pub async fn move_mouse(&mut self) {
        self.send(MouseReport {
            buttons: MOUSE_BUTTON_1,
            x: 10,
            y: 10,
        })
        .await;
    }
}

/// Relay adapter that presents the mock mouse to the HID-I2C service as a [`HidDevice`].
pub struct MockMouseHidRelay<'d> {
    service: MockMouseService<'d>,
    descriptor: HidReportDescriptor<'static>,
}

impl<'d> MockMouseHidRelay<'d> {
    pub fn new(service: MockMouseService<'d>) -> Self {
        Self {
            service,
            descriptor: HidReportDescriptor::new(MOUSE_HID_REPORT_DESCRIPTOR)
                .expect("mouse HID report descriptor should be valid"),
        }
    }
}

impl embedded_services::relay::hid::HidDevice for MockMouseHidRelay<'_> {
    type InputReportMaxSize = typenum::U3;
    type OutputReportMaxSize = typenum::U0;
    type FeatureReportMaxSize = typenum::U0;

    const MAX_REPORT_COUNT: u8 = 3;

    fn report_descriptor(&self) -> &HidReportDescriptor<'_> {
        &self.descriptor
    }

    async fn process_get_report<R>(
        &mut self,
        _report_type: GetHidReportType,
        report_id: ReportId,
        process_report: impl AsyncFnOnce(GetHidReport<'_>) -> R,
    ) -> Result<R, HidError> {
        info!("Received command to get report with ID {:?}", report_id);
        match report_id {
            ReportId(REPORTID_MOUSE) => {
                let report = MouseReport::default();
                Ok(process_report(GetHidReport::Input(HidReport::new(report_id, report.as_bytes()))).await)
            }
            _ => {
                info!("Report ID {:?} not recognized", report_id);
                Err(HidError::TriggerReset)
            }
        }
    }

    async fn set_report(&mut self, report: &SetHidReport<'_>) -> Result<(), HidError> {
        match report {
            SetHidReport::Output(r) => info!("Received command to set output report with ID {:?}", r.id()),
            SetHidReport::Feature(r) => info!("Received command to set feature report with ID {:?}", r.id()),
        }
        info!("SET_REPORT NOT IMPLEMENTED");
        Ok(())
    }

    async fn wait_for_input_report(&mut self) {
        // `receive()` only peeks at the front slot - it doesn't treat the sample as consumed until `receive_done()` is called.
        // Therefore, we can do this to wait until a report is ready, and then immediately drop it without losing the report.
        let _ = self.service.receiver().receive().await;
    }

    async fn process_next_input_report<R>(
        &mut self,
        process_report: impl AsyncFnOnce(HidReport<'_>) -> R,
    ) -> Result<R, HidError> {
        // Borrow the next report out of the channel and lend it to the transport for the duration of `process_report`.
        // This is probably unnecessary for mice because of how small the reports are, but it demonstrates the technique.
        // For a mouse it may make more sense to use a traditional Channel and just copy the 3 bytes around.
        let slot = self.service.receiver().receive().await;
        let result = process_report(HidReport::new(ReportId(REPORTID_MOUSE), slot.as_bytes())).await;

        // The transport is done with the borrow, so return the slot to the producer.  This is required by zerocopy_channel.
        self.service.receiver().receive_done();
        Ok(result)
    }

    fn has_pending_input_report(&mut self) -> bool {
        !self.service.receiver().is_empty()
    }

    async fn set_power_state(&mut self, state: HidDevicePowerState) -> Result<(), HidError> {
        info!("Received command to set power state to {:?}", state);
        Ok(())
    }

    async fn reset(&mut self) {
        info!("Received reset command");
        self.service.receiver().clear();
    }
}
