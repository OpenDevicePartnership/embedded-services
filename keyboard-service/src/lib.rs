//! Keyboard Service
//!
//! For users with basic GPIO key matrix needs, consider using the provided `GpioKeyboard`.
//!
//! Otherwise, users may manually implement the `HidKeyboard` trait for custom scanning logic
//! or hardware-implemented key scanners.
#![no_std]

pub mod gpio_kb;
pub mod hid_kb;

use embedded_services::buffer::SharedRef;
use embedded_services::hid;

pub const HID_KB_ID: hid::DeviceId = hid::DeviceId(0);

/// HID keyboard error.
pub enum KeyboardError {
    /// Rollover occurred
    Rollover,
    /// Scan error (e.g. failed to drive GPIO)
    Scan,
    /// Ghosting detected
    Ghosting,
    /// Command error
    Command,
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

/// Initialize the keyboard service given keyboard's HID configuration.
///
/// The user must also ensure the `impl_hid_kb_tasks!` macro is called to implement additional generic
/// tasks and then manually spawn them. E.g.:
///
/// ```rust,ignore
/// impl_hid_kb_tasks!(MyKeyboardType, MyI2cSlaveType, MyInterruptPinType);
/// spawner.must_spawn(keyboard_task(my_keyboard));
/// spawner.must_spawn(reports_task(my_interrupt_pin));
/// spawner.must_spawn(host_requests_task(my_i2c_slave));
/// ```
pub async fn init(
    spawner: embassy_executor::Spawner,
    hid_descriptor: hid::Descriptor,
    report_descriptor: &'static [u8],
    reg_file: hid::RegisterFile,
) {
    embedded_services::hid::init();
    hid_kb::init(spawner, hid_descriptor, report_descriptor, reg_file).await
}

// Since tasks cannot be generic, rely on this user called macro to supply the explicit type information needed
#[macro_export]
macro_rules! impl_hid_kb_tasks {
    ($hid_kb_ty:ty, $i2c_slave_ty:ty, $kb_int_ty:ty) => {
        #[embassy_executor::task]
        pub async fn keyboard_task(hid_kb: $hid_kb_ty) {
            keyboard_service::hid_kb::handle_keyboard(hid_kb).await
        }

        #[embassy_executor::task]
        pub async fn reports_task(kb_int: $kb_int_ty) {
            keyboard_service::hid_kb::handle_reports(kb_int).await
        }

        #[embassy_executor::task]
        async fn host_requests_task(kb_i2c: $i2c_slave_ty) {
            // Revisit: Make this buffer size configurable?
            embedded_services::define_static_buffer!(hid_buf, u8, [0u8; 256]);
            let buf = hid_buf::get_mut().expect("Must not already be borrowed mutably");

            // In this macro since static items cannot be generic either
            static HOST: ::static_cell::StaticCell<hid_service::i2c::Host<$i2c_slave_ty>> =
                ::static_cell::StaticCell::new();
            let host = hid_service::i2c::Host::new(keyboard_service::HID_KB_ID, kb_i2c, buf);
            let host = HOST.init(host);

            keyboard_service::hid_kb::handle_host_requests(host).await;
        }
    };
}
