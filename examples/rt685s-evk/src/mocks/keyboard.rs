//! Mock HID keyboard device used by the `mock_i2c_keyboard` example.
//!
//! Unlike the mouse mock, the keyboard copies each report out of a regular channel, which is simpler,
//! but can be costly for larger reports.

use defmt::info;
use embedded_services::relay::hid::*;
use embedded_services::warn;
use zerocopy::{FromBytes, IntoBytes};

/// Implicit report ID used by the standalone keyboard: no report-ID item appears in
/// [`KEYBOARD_HID_REPORT_DESCRIPTOR_NO_ID`], so the transport service defaults to report ID 0.
pub const REPORTID_KEYBOARD_NONE: u8 = 0;

/// Explicit report ID baked into [`KEYBOARD_HID_REPORT_DESCRIPTOR_WITH_ID`], used when the keyboard
/// is composed into an aggregate device (where every report ID must be explicit).
pub const REPORTID_KEYBOARD_AGGREGATE: u8 = 1;

// This is adapted from the example keyboard HID descriptor packaged with the DT.exe tool / https://learn.microsoft.com/en-us/windows-hardware/design/component-guidelines/keyboard-collection-report-descriptor

/// Keyboard report descriptor WITHOUT an explicit report ID. The transport service falls back to
/// report ID [`REPORTID_KEYBOARD_NONE`]. Suitable for a standalone keyboard that only exposes one
/// report.
#[rustfmt::skip]
pub const KEYBOARD_HID_REPORT_DESCRIPTOR_NO_ID: &[u8] = &[
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

/// Keyboard report descriptor WITH an explicit report ID of [`REPORTID_KEYBOARD_AGGREGATE`]. Used
/// when composing the keyboard into an aggregate device, where report IDs must be explicit so they
/// can be renumbered to avoid collisions.
#[rustfmt::skip]
pub const KEYBOARD_HID_REPORT_DESCRIPTOR_WITH_ID: &[u8] = &[
    0x05, 0x01, // USAGE_PAGE (Generic Desktop)
    0x09, 0x06, // USAGE (Keyboard)
    0xa1, 0x01, // COLLECTION (Application)
    0x85, REPORTID_KEYBOARD_AGGREGATE, //   REPORT_ID
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
#[derive(Debug, Clone, Copy, Default, defmt::Format, zerocopy::FromBytes, zerocopy::IntoBytes, zerocopy::Immutable)]
pub struct KeyboardInputReport {
    /// Left Ctrl .. Right GUI
    pub modifiers: u8,

    /// Reserved byte required by boot keyboard format
    pub reserved: u8,

    /// Up to 6 simultaneous key usages
    pub keys: [u8; 6],
}

#[repr(u8)]
#[derive(num_enum::IntoPrimitive, num_enum::TryFromPrimitive, Debug, Clone, Copy, defmt::Format)]
pub enum KeyCode {
    NumLock = 0x53,
    A = 0x04,
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy, defmt::Format, zerocopy::FromBytes, zerocopy::IntoBytes, zerocopy::Immutable)]
pub struct KeyboardOutputReport {
    /// LED state bits
    pub leds: u8,
}

/// Depth of the keyboard input-report channel.
const KEYBOARD_CHANNEL_DEPTH: usize = 5;

/// Consumer side of the mock keyboard. Owns the channel that carries input reports and exposes a
/// method to inject key presses.
pub struct MockKeyboardService {
    channel:
        embassy_sync::channel::Channel<embedded_services::GlobalRawMutex, KeyboardInputReport, KEYBOARD_CHANNEL_DEPTH>,
}

impl Default for MockKeyboardService {
    fn default() -> Self {
        Self::new()
    }
}

impl MockKeyboardService {
    pub const fn new() -> Self {
        Self {
            channel: embassy_sync::channel::Channel::new(),
        }
    }

    pub async fn click_key(&self, key_code: KeyCode) {
        // key down
        let send_result = self.channel.try_send(KeyboardInputReport {
            modifiers: 0,
            reserved: 0,
            keys: [key_code.into(), 0, 0, 0, 0, 0],
        });

        if let Err(e) = send_result {
            warn!("Failed to send key down report: {:?}", e);
        }

        embassy_time::Timer::after(embassy_time::Duration::from_millis(15)).await;

        // key up
        let send_result = self.channel.try_send(KeyboardInputReport::default());
        if let Err(e) = send_result {
            warn!("Failed to send key up report: {:?}", e);
        }
    }

    pub fn receiver(
        &self,
    ) -> embassy_sync::channel::Receiver<
        '_,
        embedded_services::GlobalRawMutex,
        KeyboardInputReport,
        KEYBOARD_CHANNEL_DEPTH,
    > {
        self.channel.receiver()
    }
}

/// Relay adapter that presents the mock keyboard to the HID-I2C service as a [`HidDevice`].
///
pub struct MockKeyboardHidRelay<'s> {
    service: &'s MockKeyboardService,
    report_id: ReportId,
    descriptor: HidReportDescriptor<'static>,
}

impl<'s> MockKeyboardHidRelay<'s> {
    pub fn new(service: &'s MockKeyboardService) -> Self {
        Self {
            service,
            report_id: ReportId(1),
            descriptor: HidReportDescriptor::new(KEYBOARD_HID_REPORT_DESCRIPTOR_WITH_ID)
                .expect("keyboard HID report descriptor should be valid"),
        }
    }
}

impl embedded_services::relay::hid::HidDevice for MockKeyboardHidRelay<'_> {
    type InputReportMaxSize = typenum::U8;
    type OutputReportMaxSize = typenum::U1;
    type FeatureReportMaxSize = typenum::U0;

    const MAX_REPORT_COUNT: u8 = 2;

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
        if report_id == self.report_id {
            let report = KeyboardInputReport::default();
            Ok(process_report(GetHidReport::Input(HidReport::new(report_id, report.as_bytes()))).await)
        } else {
            info!("Report ID {:?} not recognized", report_id);
            Err(HidError::TriggerReset)
        }
    }

    async fn set_report(&mut self, report: &SetHidReport<'_>) -> Result<(), HidError> {
        match report {
            SetHidReport::Output(r) if r.id() == self.report_id => {
                let output_report = KeyboardOutputReport::read_from_bytes(r.data()).unwrap();
                info!("Received keyboard output report: {:?}", output_report);
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
        self.service.receiver().ready_to_receive().await
    }

    async fn process_next_input_report<R>(
        &mut self,
        process_report: impl AsyncFnOnce(HidReport<'_>) -> R,
    ) -> Result<R, HidError> {
        let input_report = self.service.receiver().receive().await;
        Ok(process_report(HidReport::new(self.report_id, input_report.as_bytes())).await)
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
