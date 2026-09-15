//! Shared test-only mocks for the HID-I2C target service.
//!
//! These deliberately model *failure* as well as success. A mock whose error type is
//! [`core::convert::Infallible`] makes the service's error-handling arms unreachable by
//! construction, so the tests can never show that they work.

#![allow(clippy::unwrap_used)]
use crate::{HardwareVersionInfo, ProductId, VendorId, VersionId};
use core::marker::PhantomData;
use embedded_services::relay::hid::{
    GetHidReport, GetHidReportType, HidDevice, HidDevicePowerState, HidError, HidReport, HidReportDescriptor, ReportId,
    SetHidReport,
};
use generic_array::ArrayLength;

/// Report descriptor with explicit output + feature report IDs, used by the wire-format tests.
pub const MOUSE_DESCRIPTOR: &[u8] = &[
    0x05, 0x01, // Usage Page (Generic Desktop)
    0x09, 0x02, // Usage (Mouse)
    0xa1, 0x01, // Collection (Application)
    0x85, 0x03, // Report ID (3)
    0x75, 0x08, // Report Size (8)
    0x95, 0x01, // Report Count (1)
    0x91, 0x02, // Output
    0x85, 0x21, // Report ID (33)
    0xb1, 0x02, // Feature
    0xc0, // End Collection
];

/// Report descriptor with a single top-level collection, so report IDs are implicit.
pub const IMPLICIT_ID_DESCRIPTOR: &[u8] = &[
    0x05, 0x01, // Usage Page (Generic Desktop)
    0x09, 0x02, // Usage (Mouse)
    0xa1, 0x01, // Collection (Application)
    0x75, 0x08, // Report Size (8)
    0x95, 0x01, // Report Count (1)
    0x81, 0x02, // Input
    0x91, 0x02, // Output
    0xb1, 0x02, // Feature
    0xc0, // End Collection
];

/// An unsolicited input report queued on the mock, ready for the service to pick up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingInputReport {
    pub id: ReportId,
    pub data: Vec<u8>,
}

/// A HID device mock that records the most recent command it received so tests can assert on it,
/// and which can be told to fail any of its fallible operations.
///
/// Generic only over the parameters that tests actually vary: the three report-size maxima
/// (the descriptor tests need 1-byte maxima to reach the oversize-rejection paths, the
/// wire-format tests need room for multi-byte reports) and the declared descriptor-length
/// bound, which one test drives below the real descriptor length to check that the device is
/// held to its own contract. `MAX_REPORT_COUNT` is consulted by nothing under test, so it is
/// fixed rather than being another knob to thread through.
pub struct MockHidDevice<In, Out, Feat, const DESC_LEN: usize = 64>
where
    In: ArrayLength,
    Out: ArrayLength,
    Feat: ArrayLength,
{
    descriptor: HidReportDescriptor<'static>,
    pub power_state: Option<HidDevicePowerState>,
    pub report_id: Option<ReportId>,
    pub report_data: [u8; 8],
    pub report_len: usize,
    pub feature_report: bool,
    pub reset_count: usize,

    /// Queued unsolicited input report. Drives [`HidDevice::has_pending_input_report`] and is
    /// consumed by [`HidDevice::process_next_input_report`].
    pub pending_input: Option<PendingInputReport>,

    /// Payload handed back by [`HidDevice::process_get_report`].
    pub get_report_payload: Vec<u8>,

    /// When set, the corresponding operation fails with this error instead of succeeding.
    pub fail_get_report: Option<HidError>,
    pub fail_set_report: Option<HidError>,
    pub fail_set_power: Option<HidError>,

    _phantom: PhantomData<(In, Out, Feat)>,
}

impl<In, Out, Feat, const DESC_LEN: usize> MockHidDevice<In, Out, Feat, DESC_LEN>
where
    In: ArrayLength,
    Out: ArrayLength,
    Feat: ArrayLength,
{
    pub fn new(descriptor: &'static [u8]) -> Self {
        Self {
            descriptor: HidReportDescriptor::new(descriptor).unwrap(),
            power_state: None,
            report_id: None,
            report_data: [0; 8],
            report_len: 0,
            feature_report: false,
            reset_count: 0,
            pending_input: None,
            get_report_payload: vec![0x5a],
            fail_get_report: None,
            fail_set_report: None,
            fail_set_power: None,
            _phantom: PhantomData,
        }
    }

    /// Queue an unsolicited input report for the service to collect.
    pub fn with_pending_input(mut self, id: ReportId, data: &[u8]) -> Self {
        self.pending_input = Some(PendingInputReport {
            id,
            data: Vec::from(data),
        });
        self
    }

    /// Set the payload that `process_get_report` will hand back.
    pub fn with_get_report_payload(mut self, data: &[u8]) -> Self {
        self.get_report_payload = Vec::from(data);
        self
    }
}

impl<In, Out, Feat, const DESC_LEN: usize> HidDevice for MockHidDevice<In, Out, Feat, DESC_LEN>
where
    In: ArrayLength,
    Out: ArrayLength,
    Feat: ArrayLength,
{
    type InputReportMaxSize = In;
    type OutputReportMaxSize = Out;
    type FeatureReportMaxSize = Feat;

    const MAX_REPORT_COUNT: u8 = 8;
    const MAX_DESCRIPTOR_LEN: usize = DESC_LEN;

    fn report_descriptor(&self) -> &HidReportDescriptor<'_> {
        &self.descriptor
    }

    async fn process_get_report<R>(
        &mut self,
        report_type: GetHidReportType,
        report_id: ReportId,
        process_report: impl AsyncFnOnce(GetHidReport<'_>) -> R,
    ) -> Result<R, HidError> {
        if let Some(error) = self.fail_get_report {
            return Err(error);
        }

        let report = HidReport::new(report_id, &self.get_report_payload);
        let report = match report_type {
            GetHidReportType::Input => GetHidReport::Input(report),
            GetHidReportType::Feature => GetHidReport::Feature(report),
        };
        Ok(process_report(report).await)
    }

    async fn set_report(&mut self, report: &SetHidReport<'_>) -> Result<(), HidError> {
        if let Some(error) = self.fail_set_report {
            return Err(error);
        }

        self.report_id = Some(report.id());
        self.report_len = report.data().len();
        self.report_data
            .get_mut(..self.report_len)
            .ok_or(HidError::TriggerReset)?
            .copy_from_slice(report.data());
        self.feature_report = matches!(report, SetHidReport::Feature(_));
        Ok(())
    }

    async fn wait_for_input_report(&mut self) {
        if self.pending_input.is_some() {
            return;
        }
        core::future::pending().await
    }

    fn has_pending_input_report(&mut self) -> bool {
        self.pending_input.is_some()
    }

    async fn process_next_input_report<R>(
        &mut self,
        process_report: impl AsyncFnOnce(HidReport<'_>) -> R,
    ) -> Result<R, HidError> {
        let pending = self.pending_input.take().ok_or(HidError::TriggerReset)?;
        Ok(process_report(HidReport::new(pending.id, &pending.data)).await)
    }

    async fn set_power_state(&mut self, state: HidDevicePowerState) -> Result<(), HidError> {
        if let Some(error) = self.fail_set_power {
            return Err(error);
        }

        self.power_state = Some(state);
        Ok(())
    }

    async fn reset(&mut self) {
        self.reset_count += 1;
        self.power_state = Some(HidDevicePowerState::On);
        self.pending_input = None;
    }
}

/// Wire-format recording device: 8-byte report maxima with explicit output + feature reports.
pub type RecordingHidDevice = MockHidDevice<typenum::U8, typenum::U8, typenum::U8>;

/// Descriptor-sizing device: single-byte report maxima to exercise the oversize-rejection paths.
pub type DescriptorHidDevice = MockHidDevice<typenum::U1, typenum::U1, typenum::U1>;

/// A device that under-declares `MAX_DESCRIPTOR_LEN` relative to the descriptor it actually
/// returns, violating the contract in [`HidDevice::MAX_DESCRIPTOR_LEN`].
pub type UnderDeclaredDescriptorDevice = MockHidDevice<typenum::U8, typenum::U8, typenum::U8, 4>;

/// Constructs an [`UnderDeclaredDescriptorDevice`] backed by the 19-byte [`MOUSE_DESCRIPTOR`].
pub fn under_declared_descriptor_device() -> UnderDeclaredDescriptorDevice {
    MockHidDevice::new(MOUSE_DESCRIPTOR)
}

/// Constructs a [`RecordingHidDevice`] backed by [`MOUSE_DESCRIPTOR`].
pub fn recording_device() -> RecordingHidDevice {
    MockHidDevice::new(MOUSE_DESCRIPTOR)
}

/// Constructs a [`RecordingHidDevice`] whose descriptor uses implicit report IDs.
pub fn implicit_id_device() -> RecordingHidDevice {
    MockHidDevice::new(IMPLICIT_ID_DESCRIPTOR)
}

/// Constructs a [`DescriptorHidDevice`] backed by the provided report descriptor.
pub fn descriptor_device(descriptor: &'static [u8]) -> DescriptorHidDevice {
    MockHidDevice::new(descriptor)
}

/// A fixed set of hardware identifiers used across the descriptor and service tests.
pub fn hardware_version_info() -> HardwareVersionInfo {
    HardwareVersionInfo {
        vendor_id: VendorId::new(0x1234).unwrap(),
        product_id: ProductId(0x5678),
        version_id: VersionId(0x0100),
    }
}

/// Level driven onto a GPIO by [`RecordingPin`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinLevel {
    Low,
    High,
}

/// An [`embedded_hal::digital::OutputPin`] that records every level it is driven to, and which
/// can be told to start failing.
///
/// Recording matters: `AttnPinHandler` tracks assertion state in its own field, so a test that
/// only asserts on `asserted()` passes even when the GPIO is never touched. Checking `levels`
/// is what actually pins the hardware behaviour down.
#[derive(Debug, Default)]
pub struct RecordingPin {
    pub levels: Vec<PinLevel>,
    /// Once this many transitions have been recorded, every later call fails.
    pub fail_after: Option<usize>,
}

impl RecordingPin {
    pub fn new() -> Self {
        Self::default()
    }

    /// A pin that starts failing after `n` successful transitions.
    pub fn failing_after(n: usize) -> Self {
        Self {
            levels: Vec::new(),
            fail_after: Some(n),
        }
    }

    /// The most recently driven level, if the pin has been driven at all.
    pub fn level(&self) -> Option<PinLevel> {
        self.levels.last().copied()
    }

    fn drive(&mut self, level: PinLevel) -> Result<(), embedded_hal::digital::ErrorKind> {
        if self.fail_after.is_some_and(|limit| self.levels.len() >= limit) {
            return Err(embedded_hal::digital::ErrorKind::Other);
        }
        self.levels.push(level);
        Ok(())
    }
}

impl embedded_hal::digital::ErrorType for RecordingPin {
    type Error = embedded_hal::digital::ErrorKind;
}

impl embedded_hal::digital::OutputPin for RecordingPin {
    fn set_low(&mut self) -> Result<(), Self::Error> {
        self.drive(PinLevel::Low)
    }

    fn set_high(&mut self) -> Result<(), Self::Error> {
        self.drive(PinLevel::High)
    }
}
