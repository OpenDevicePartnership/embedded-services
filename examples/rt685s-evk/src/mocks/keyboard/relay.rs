use super::interface::*;
use defmt::info;
use embedded_services::relay::hid::*;
use zerocopy::{FromBytes, IntoBytes};

/// Implicit report ID used by the standalone keyboard: no report-ID item appears in
/// [`KEYBOARD_HID_REPORT_DESCRIPTOR_NO_ID`], so the transport service defaults to report ID 0.
const REPORTID_KEYBOARD_NO_ID: ReportId = ReportId(0);

// This is adapted from the example keyboard HID descriptor packaged with the DT.exe tool / https://learn.microsoft.com/en-us/windows-hardware/design/component-guidelines/keyboard-collection-report-descriptor

/// Keyboard report descriptor WITHOUT an explicit report ID. The transport service falls back to
/// report ID [`REPORTID_KEYBOARD_NO_ID`]. Suitable for a standalone keyboard that only exposes one
/// report.
#[rustfmt::skip]
const KEYBOARD_HID_REPORT_DESCRIPTOR_NO_ID: &[u8] = &[
    0x05, 0x01, // USAGE_PAGE (Generic Desktop)
    0x09, 0x06, // USAGE (Keyboard)
    0xa1, 0x01, // COLLECTION (Application)
    0x05, 0x07, //   USAGE_PAGE (Keyboard)
    0x19, 0xe0, //   USAGE_MINIMUM (Keyboard LeftControl)
    0x29, 0xe7, //   USAGE_MAXIMUM (Keyboard Right GUI)
    0x15, 0x00, //   LOGICAL_MINIMUM (0)
    0x25, 0x01, //   LOGICAL_MAXIMUM (1)
    0x75, 0x01, //   REPORT_SIZE (1)
    0x95, 0x08, //   REPORT_COUNT (8)
    0x81, 0x02, //   INPUT (Data,Var,Abs)
    0x95, 0x01, //   REPORT_COUNT (1)
    0x75, 0x08, //   REPORT_SIZE (8)
    0x81, 0x03, //   INPUT (Cnst,Var,Abs)
    0x95, 0x05, //   REPORT_COUNT (5)
    0x75, 0x01, //   REPORT_SIZE (1)
    0x05, 0x08, //   USAGE_PAGE (LEDs)
    0x19, 0x01, //   USAGE_MINIMUM (Num Lock)
    0x29, 0x05, //   USAGE_MAXIMUM (Kana)
    0x91, 0x02, //   OUTPUT (Data,Var,Abs)
    0x95, 0x01, //   REPORT_COUNT (1)
    0x75, 0x03, //   REPORT_SIZE (3)
    0x91, 0x03, //   OUTPUT (Cnst,Var,Abs)
    0x95, 0x06, //   REPORT_COUNT (6)
    0x75, 0x08, //   REPORT_SIZE (8)
    0x15, 0x00, //   LOGICAL_MINIMUM (0)
    0x25, 0x65, //   LOGICAL_MAXIMUM (101)
    0x05, 0x07, //   USAGE_PAGE (Keyboard)
    0x19, 0x00, //   USAGE_MINIMUM (Reserved (no event indicated))
    0x29, 0x65, //   USAGE_MAXIMUM (Keyboard Application)
    0x81, 0x00, //   INPUT (Data,Ary,Abs)
    0xc0, // END_COLLECTION
];

#[repr(C, packed)]
#[derive(Debug, Clone, Copy, defmt::Format, zerocopy::FromBytes, zerocopy::IntoBytes, zerocopy::Immutable)]
struct KeyboardOutputReport {
    /// LED state bits
    pub leds: u8,
}

/// Relay adapter that presents the mock keyboard to the HID-I2C service as a [`HidDevice`].
///
pub struct MockKeyboardHidRelay<'s, Service: KeyboardService<'s>> {
    service: &'s mut Service,
    subscriber: embassy_sync::pubsub::DynSubscriber<'s, KeyboardInputReport>,

    descriptor: HidReportDescriptor<'static>,

    pending_input_report: Option<KeyboardInputReport>,
}

impl<'s, Service: KeyboardService<'s>> MockKeyboardHidRelay<'s, Service> {
    pub fn new(service: &'s mut Service) -> Self {
        let subscriber = service
            .subscriber()
            .expect("keyboard service didn't have enough subscriber slots to create a relay");
        Self {
            service,
            subscriber,
            descriptor: HidReportDescriptor::new(KEYBOARD_HID_REPORT_DESCRIPTOR_NO_ID)
                .expect("keyboard HID report descriptor should be valid"),
            pending_input_report: None,
        }
    }
}

impl<'s, Service: KeyboardService<'s>> embedded_services::relay::hid::HidDevice for MockKeyboardHidRelay<'s, Service> {
    type InputReportMaxSize = typenum::U8;
    type OutputReportMaxSize = typenum::U1;
    type FeatureReportMaxSize = typenum::U0;

    const MAX_REPORT_COUNT: u8 = 1;
    const MAX_DESCRIPTOR_LEN: usize = KEYBOARD_HID_REPORT_DESCRIPTOR_NO_ID.len();

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
        if report_id == REPORTID_KEYBOARD_NO_ID {
            let report = KeyboardInputReport::default();
            Ok(process_report(GetHidReport::Input(HidReport::new(report_id, report.as_bytes()))).await)
        } else {
            info!("Report ID {:?} not recognized", report_id);
            Err(HidError::TriggerReset)
        }
    }

    async fn set_report(&mut self, report: &SetHidReport<'_>) -> Result<(), HidError> {
        match report {
            SetHidReport::Output(r) if r.id() == REPORTID_KEYBOARD_NO_ID => {
                let output_report = KeyboardOutputReport::read_from_bytes(r.data()).unwrap();
                self.service.set_led(output_report.leds).await;
            }
            SetHidReport::Output(r) => {
                info!("Report ID {:?} not recognized", r.id());
                return Err(HidError::TriggerReset);
            }
            SetHidReport::Feature(r) => info!("Received command to set feature report with ID {:?}", r.id()),
        }
        Ok(())
    }

    async fn wait_for_input_report(&mut self) {
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
            REPORTID_KEYBOARD_NO_ID,
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
