//! HID relay adapter for keyboard services.

use embassy_sync::pubsub::DynSubscriber;
use embedded_services::error;
use embedded_services::relay::hid::{
    GetHidReport, GetHidReportType, HidDescriptorError, HidDevice, HidDevicePowerState, HidError, HidReport,
    HidReportDescriptor, ReportId, SetHidReport,
};

use crate::interface::{KeyboardInputReport, KeyboardPowerState, KeyboardService, LedFlags};

const REPORT_ID: ReportId = ReportId(0);

#[rustfmt::skip]
const REPORT_DESCRIPTOR: &[u8] = &[
    // Usage Page (Generic Desktop Ctrls)
    0x05, 0x01,
    // Usage (Keyboard)
    0x09, 0x06,
    // Collection (Application)
    0xA1, 0x01,
    // Usage Page (Keypad)
    0x05, 0x07,
    // Usage Minimum (0xE0)
    0x19, 0xE0,
    // Usage Maximum (0xE7)
    0x29, 0xE7,
    // Logical Minimum (0)
    0x15, 0x00,
    // Logical Maximum (1)
    0x25, 0x01,
    // Report Size (1)
    0x75, 0x01,
    // Report Count (8) (8 modifier keys represented by single bit)
    0x95, 0x08,
    // Input (Data,Var,Abs,No Wrap,Linear,Preferred State,No Null Position)
    0x81, 0x02,
    // Usage Minimum (0x00)
    0x19, 0x00,
    // Usage Maximum (0x91)
    0x29, 0x91,
    // Logical Maximum (255)
    0x26, 0xFF, 0x00,
    // Report Size (8)
    0x75, 0x08,
    // Report Count (6) (Keyberon only supports 6 keys)
    0x95, 0x06,
    // Input (Data,Array,Abs,No Wrap,Linear,Preferred State,No Null Position)
    0x81, 0x00,
    // LED report
    // Usage Page (LEDs)
    0x05, 0x08,
    // Usage Minimum (Num Lock)
    0x19, 0x01,
    // Usage Maximum (Scroll Lock)
    0x29, 0x03,
    // Report Size (1)
    0x75, 0x01,
    // Report Count (3)
    0x95, 0x03,
    // Logical Maximum (1)
    0x25, 0x01,
    // Output (Data,Var,Abs,No Wrap,Linear,Preferred State,No Null Position)
    0x91, 0x02,
    // Report Count (5)
    0x95, 0x05,
    // Output (Const,Array,Abs,No Wrap,Linear,Preferred State,No Null Position)
    0x91, 0x01,
    // End Collection
    0xC0,
];

/// Adapter that presents a [`KeyboardService`] as a HID device.
pub struct KeyboardHidRelay<'s, S: KeyboardService<'s>> {
    service: &'s mut S,
    subscriber: DynSubscriber<'s, KeyboardInputReport>,
    descriptor: HidReportDescriptor<'static>,

    // Keyboard report that has been received by our subscriber but not accepted by the HID relay yet
    pending_input_report: Option<KeyboardInputReport>,

    // The most recent input report that was accepted by the HID relay.  This is our understanding of the
    // currently-depressed keys and is used to respond to GetReport requests.
    last_report: KeyboardInputReport,
}

#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyboardHidRelayError {
    /// The keyboard service has no subscriber slot available for the HID relay.
    NoSubscriberSlotAvailable,

    /// The keyboard service's HID report descriptor is invalid.
    InvalidReportDescriptor(HidDescriptorError),
}

impl From<HidDescriptorError> for KeyboardHidRelayError {
    fn from(err: HidDescriptorError) -> Self {
        Self::InvalidReportDescriptor(err)
    }
}

impl<'s, S: KeyboardService<'s>> KeyboardHidRelay<'s, S> {
    /// Creates a HID relay for a keyboard service.
    pub fn new(service: &'s mut S) -> Result<Self, KeyboardHidRelayError> {
        let subscriber = service
            .subscriber()
            .map_err(|_| KeyboardHidRelayError::NoSubscriberSlotAvailable)?;

        Ok(Self {
            service,
            subscriber,
            descriptor: HidReportDescriptor::new(REPORT_DESCRIPTOR)?,
            pending_input_report: None,
            last_report: KeyboardInputReport::default(),
        })
    }
}

impl<'s, S: KeyboardService<'s>> HidDevice for KeyboardHidRelay<'s, S> {
    type InputReportMaxSize = typenum::U8;
    type OutputReportMaxSize = typenum::U1;
    type FeatureReportMaxSize = typenum::U0;

    const MAX_REPORT_COUNT: u8 = 1;
    const MAX_DESCRIPTOR_LEN: usize = REPORT_DESCRIPTOR.len();

    fn report_descriptor(&self) -> &HidReportDescriptor<'_> {
        &self.descriptor
    }

    async fn process_get_report<R>(
        &mut self,
        report_type: GetHidReportType,
        report_id: ReportId,
        process_report: impl AsyncFnOnce(GetHidReport<'_>) -> R,
    ) -> Result<R, HidError> {
        match (report_type, report_id) {
            (GetHidReportType::Input, REPORT_ID) => Ok(process_report(GetHidReport::Input(HidReport::new(
                report_id,
                self.last_report.as_bytes(),
            )))
            .await),
            _ => Err(HidError::TriggerReset),
        }
    }

    async fn set_report(&mut self, report: &SetHidReport<'_>) -> Result<(), HidError> {
        match report {
            SetHidReport::Output(report) if report.id() == REPORT_ID => {
                let flags = LedFlags::from_bits_retain(report.data().first().copied().unwrap_or(0));
                if self.service.set_leds(flags).await.is_err() {
                    error!("Failed to set keyboard LEDs");
                }
            }
            SetHidReport::Output(_) | SetHidReport::Feature(_) => {}
        }

        Ok(())
    }

    async fn wait_for_input_report(&mut self) {
        if self.pending_input_report.is_none() {
            self.pending_input_report = Some(self.subscriber.next_message_pure().await);
        }
    }

    fn has_pending_input_report(&mut self) -> bool {
        self.pending_input_report.is_some() || !self.subscriber.is_empty()
    }

    async fn process_next_input_report<R>(
        &mut self,
        process_report: impl AsyncFnOnce(HidReport<'_>) -> R,
    ) -> Result<R, HidError> {
        self.wait_for_input_report().await;

        // Panic safety: wait_for_input_report always populates the pending report, so take() will never fail.
        #[allow(clippy::expect_used)]
        let report = self
            .pending_input_report
            .take()
            .expect("wait_for_input_report always populates the pending report");
        self.last_report = report;

        Ok(process_report(HidReport::new(REPORT_ID, self.last_report.as_bytes())).await)
    }

    async fn set_power_state(&mut self, state: HidDevicePowerState) -> Result<(), HidError> {
        self.service.set_power_state(match state {
            HidDevicePowerState::On => KeyboardPowerState::On,
            HidDevicePowerState::Sleep => KeyboardPowerState::Sleep,
            HidDevicePowerState::Off => KeyboardPowerState::Off,
        });
        Ok(())
    }

    async fn reset(&mut self) {
        self.pending_input_report = None;
        self.subscriber.clear();
    }
}
