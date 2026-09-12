//! Shared test-only mocks for the HID-I2C target service.

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

/// A HID device mock that records the most recent command it received so tests can assert on it.
///
/// It is generic over the report-size / report-count / descriptor-length parameters so it can serve
/// both the descriptor-sizing tests (which need small maxima to exercise the oversize-rejection paths)
/// and the wire-format tests (which need room for multi-byte reports).
pub struct MockHidDevice<In, Out, Feat, const REPORT_COUNT: u8, const DESC_LEN: usize>
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
    _phantom: PhantomData<(In, Out, Feat)>,
}

impl<In, Out, Feat, const REPORT_COUNT: u8, const DESC_LEN: usize> MockHidDevice<In, Out, Feat, REPORT_COUNT, DESC_LEN>
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
            _phantom: PhantomData,
        }
    }
}

impl<In, Out, Feat, const REPORT_COUNT: u8, const DESC_LEN: usize> HidDevice
    for MockHidDevice<In, Out, Feat, REPORT_COUNT, DESC_LEN>
where
    In: ArrayLength,
    Out: ArrayLength,
    Feat: ArrayLength,
{
    type InputReportMaxSize = In;
    type OutputReportMaxSize = Out;
    type FeatureReportMaxSize = Feat;

    const MAX_REPORT_COUNT: u8 = REPORT_COUNT;
    const MAX_DESCRIPTOR_LEN: usize = DESC_LEN;

    fn report_descriptor(&self) -> &HidReportDescriptor<'_> {
        &self.descriptor
    }

    async fn process_get_report<R>(
        &mut self,
        _report_type: GetHidReportType,
        report_id: ReportId,
        process_report: impl AsyncFnOnce(GetHidReport<'_>) -> R,
    ) -> Result<R, HidError> {
        Ok(process_report(GetHidReport::Feature(HidReport::new(report_id, &[0x5a]))).await)
    }

    async fn set_report(&mut self, report: &SetHidReport<'_>) -> Result<(), HidError> {
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
        core::future::pending().await
    }

    fn has_pending_input_report(&mut self) -> bool {
        false
    }

    async fn process_next_input_report<R>(
        &mut self,
        process_report: impl AsyncFnOnce(HidReport<'_>) -> R,
    ) -> Result<R, HidError> {
        Ok(process_report(HidReport::new(ReportId(0), &[])).await)
    }

    async fn set_power_state(&mut self, state: HidDevicePowerState) -> Result<(), HidError> {
        self.power_state = Some(state);
        Ok(())
    }

    async fn reset(&mut self) {
        self.reset_count += 1;
        self.power_state = Some(HidDevicePowerState::On);
    }
}

/// Wire-format recording device: 8-byte report maxima with explicit output + feature reports.
pub type RecordingHidDevice = MockHidDevice<typenum::U8, typenum::U8, typenum::U8, 2, 19>;

/// Descriptor-sizing device: single-byte report maxima to exercise the oversize-rejection paths.
pub type DescriptorHidDevice = MockHidDevice<typenum::U1, typenum::U1, typenum::U1, 1, 32>;

/// Constructs a [`RecordingHidDevice`] backed by [`MOUSE_DESCRIPTOR`].
pub fn recording_device() -> RecordingHidDevice {
    MockHidDevice::new(MOUSE_DESCRIPTOR)
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
