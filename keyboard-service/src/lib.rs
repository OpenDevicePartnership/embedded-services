//! Keyboard Service
//!
//! For users with basic GPIO key matrix needs, consider using the provided `GpioKeyboard`.
//!
//! Otherwise, users may manually implement the `HidKeyboard` trait for custom scanning logic
//! or hardware-implemented key scanners.
#![no_std]
#![allow(clippy::expect_used)]
#![allow(clippy::indexing_slicing)]
#![allow(clippy::panic_in_result_fn)]
#![allow(clippy::unwrap_used)]

pub mod gpio_kb;
pub mod hid_kb;
pub mod task;

use embedded_services::buffer::SharedRef;
use embedded_services::hid;

pub const HID_KB_ID: hid::DeviceId = hid::DeviceId(0);

/// HID keyboard error.
#[derive(Debug)]
pub enum KeyboardError {
    /// Rollover occurred
    Rollover,
    /// Scan error (e.g. failed to drive GPIO)
    Scan,
    /// Ghosting detected
    Ghosting,
    /// Command error
    Command,
    /// Buffer error
    Buffer(embedded_services::buffer::Error),
}

/// A slice of a HID report.
///
/// This should only contain a single key modifiers byte followed by KRO usage codes.
/// The HID backend will add the appropriate header for underlying protocol (e.g. HID over i2c header).
pub struct HidReportSlice<'a>(&'a [u8]);

impl HidReportSlice<'_> {
    /// Returns the HID report as a raw byte slice.
    pub fn as_bytes(&self) -> &[u8] {
        self.0
    }
}

/// Represents a HID-aware keyboard.
///
/// This should be implemented on a struct and passed to the keyboard service initialization
/// if not using the provided `GpioKeyboard`.
pub trait HidKeyboard {
    /// Returns the HID descriptor for the keyboard.
    fn hid_descriptor(&self) -> hid::Descriptor;

    /// Returns the report descriptor for the keyboard as a static byte slice.
    //
    // Revisit: To support this not being static would require a bit of refactoring,
    // since the slice gets passed to one task while Self gets passed to another, which would
    // require a bit of lifetime management if this were not static.
    fn report_descriptor(&self) -> &'static [u8];

    /// Returns the register file for the keyboard.
    fn register_file(&self) -> hid::RegisterFile;

    /// Performs a key scan, yielding when a report is available.
    ///
    /// If no report is available, this should not yield unless dictated by the idle frequency.
    ///
    /// The format of the report depends on the protocol (Boot vs Report) as well as underlying
    /// transport protocol (I2C vs USB, for example).
    ///
    /// # Cancel Safety
    ///
    /// The implementation MUST be cancel safe as the HID backend may cancel to service an incoming command.
    fn scan(&mut self) -> impl core::future::Future<Output = Result<HidReportSlice<'_>, KeyboardError>>;

    /// Resets the keyboard to initial state.
    fn reset(&mut self) -> impl core::future::Future<Output = Result<(), KeyboardError>>;

    /// Sets the power state of the keyboard.
    ///
    /// In sleep state, keyboard should not yield new input reports.
    fn set_power_state(
        &mut self,
        power_state: hid::PowerState,
    ) -> impl core::future::Future<Output = Result<(), KeyboardError>>;

    /// Sets the frequency the keyboard should yield reports even if no new events have occurred.
    ///
    /// A frequency of `ReportFreq::Infinite` should result in the keyboard ONLY yielding reports
    /// when new events have occurred.
    fn set_idle(
        &mut self,
        report_id: hid::ReportId,
        report_freq: hid::ReportFreq,
    ) -> impl core::future::Future<Output = Result<(), KeyboardError>>;

    /// Gets the idle frequency of the keyboard.
    fn get_idle(&self, report_id: hid::ReportId) -> hid::ReportFreq;

    /// Sets the protocol (Boot vs Report) of the keyboard.
    fn set_protocol(
        &mut self,
        protocol: hid::Protocol,
    ) -> impl core::future::Future<Output = Result<(), KeyboardError>>;

    /// Gets the protocol (Boot vs Report) of the keyboard.
    fn get_protocol(&self) -> hid::Protocol;

    /// Perform a vendor-defined keyboard command.
    fn vendor_cmd(&mut self) -> impl core::future::Future<Output = Result<(), KeyboardError>>;

    /// Sets output or feature report for the keyboard with the given report ID.
    fn set_report(
        &mut self,
        report_type: hid::ReportType,
        report_id: hid::ReportId,
        buf: &SharedRef<'static, u8>,
    ) -> impl core::future::Future<Output = Result<(), KeyboardError>>;

    /// Gets input or feature report for the keyboard with the given report ID.
    fn get_report(&self, report_type: hid::ReportType, report_id: hid::ReportId) -> HidReportSlice<'_>;
}
