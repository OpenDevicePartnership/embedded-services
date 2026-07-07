use super::interface::*;
use defmt::info;
use embedded_services::relay::hid::*;
use zerocopy::IntoBytes;

// This is adapted from the example mouse HID descriptor packaged with the DT.exe tool / https://learn.microsoft.com/en-us/windows-hardware/design/component-guidelines/mouse-collection-report-descriptor
const REPORTID_MOUSE: u8 = 1;

#[rustfmt::skip]
const MOUSE_HID_REPORT_DESCRIPTOR: &[u8] = &[
    0x05, 0x01, // Usage Page (Generic Desktop Ctrls)
    0x09, 0x02, // Usage (Mouse)
    0xA1, 0x01, // Collection (Application)
    0x85, REPORTID_MOUSE, //   REPORT_ID (mouse report)
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

/// Relay adapter that presents the mock mouse to the HID-I2C service as a [`HidDevice`].
pub struct MockMouseHidRelay<'s, Service: MouseService<'s>> {
    _service: &'s mut Service, // This simplified example mouse doesn't include output/feature reports to control e.g. mouse DPI settings, but if they did, you'd route them through this
    subscriber: embassy_sync::pubsub::DynSubscriber<'s, MouseInputReport>,

    descriptor: HidReportDescriptor<'static>,

    pending_input_report: Option<MouseInputReport>,
}

impl<'s, Service: MouseService<'s>> MockMouseHidRelay<'s, Service> {
    pub fn new(service: &'s mut Service) -> Self {
        let subscriber = service
            .subscriber()
            .expect("mouse service didn't have enough subscriber slots to create a relay");
        Self {
            _service: service,
            subscriber,
            descriptor: HidReportDescriptor::new(MOUSE_HID_REPORT_DESCRIPTOR)
                .expect("mouse HID report descriptor should be valid"),
            pending_input_report: None,
        }
    }
}

impl<'s, Service: MouseService<'s>> embedded_services::relay::hid::HidDevice for MockMouseHidRelay<'s, Service> {
    type InputReportMaxSize = typenum::U3;
    type OutputReportMaxSize = typenum::U0;
    type FeatureReportMaxSize = typenum::U0;

    const MAX_REPORT_COUNT: u8 = 1;
    const MAX_DESCRIPTOR_LEN: usize = MOUSE_HID_REPORT_DESCRIPTOR.len();

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
                let report = MouseInputReport::default();
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
        // Peek the next report into a local slot so a report is never lost if the future driving this
        // relay is dropped by a `select`-style combinator before `process_next_input_report` runs.
        if self.pending_input_report.is_none() {
            self.pending_input_report = Some(self.subscriber.next_message_pure().await);
        }
    }

    async fn process_next_input_report<R>(
        &mut self,
        process_report: impl AsyncFnOnce(HidReport<'_>) -> R,
    ) -> Result<R, HidError> {
        self.wait_for_input_report().await;

        Ok(process_report(HidReport::new(
            ReportId(REPORTID_MOUSE),
            self.pending_input_report
                .take()
                .expect("We just forced this to be populated in wait_for_input_report()")
                .as_bytes(),
        ))
        .await)
    }

    fn has_pending_input_report(&mut self) -> bool {
        self.pending_input_report.is_some() || !self.subscriber.is_empty()
    }

    async fn set_power_state(&mut self, state: HidDevicePowerState) -> Result<(), HidError> {
        info!("Received command to set power state to {:?}", state);
        Ok(())
    }

    async fn reset(&mut self) {
        info!("Received reset command");
        self.pending_input_report = None;
        self.subscriber.clear();
    }
}
