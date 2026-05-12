//! HID relay code

use generic_array::ArrayLength;
use num_enum::TryFromPrimitive;

/// Errors that a HID device operation can fail with.
///
/// Reporting failure triggers a device-initiated reset, so callers must handle these errors explicitly
/// rather than swallowing them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum HidError {
    /// The operation has failed and a device-initiated reset should be triggered.
    TriggerReset,
}

/// Power states that the host can command a HID device to be put into.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum HidDevicePowerState {
    /// Normal operation
    On,

    /// Reduced power state, but a device that sends a report in this state can wake the host - quiesce messages if you don't want to do that
    Sleep,

    /// The device is not allowed to wake the host. This is not supported on all transports - in particular, I2C will never command a device into the off state.
    Off,
}

/// A HID report of no more than X bytes
pub struct HidReport<'buf> {
    id: ReportId,

    data: &'buf [u8],
}

impl<'buf> HidReport<'buf> {
    /// Create a new HID report from the provided data slice.
    pub fn new(id: ReportId, data: &'buf [u8]) -> Self {
        Self { id, data }
    }

    /// The report ID for this report
    pub fn id(&self) -> ReportId {
        self.id
    }

    /// The data for this report.
    pub fn data(&self) -> &'buf [u8] {
        self.data
    }
}

/// HID report types supported by the SetReport operation.
pub enum SetHidReport<'buf> {
    /// An output report
    Output(HidReport<'buf>),

    /// A feature report
    Feature(HidReport<'buf>),
}

impl<'buf> SetHidReport<'buf> {
    /// The data for this report, whatever its type.
    pub fn data(&self) -> &'buf [u8] {
        match self {
            SetHidReport::Output(report) => report.data(),
            SetHidReport::Feature(report) => report.data(),
        }
    }
}

/// A type of report that can be requested by the host
pub enum GetHidReportType {
    /// The host has requested an input report
    Input,

    /// The host has requested a feature report
    Feature,
}

/// HID report types supported by the GetReport operation.
pub enum GetHidReport<'buf> {
    /// An input report
    Input(HidReport<'buf>),

    /// A feature report
    Feature(HidReport<'buf>),
}

impl<'buf> GetHidReport<'buf> {
    /// The data for this report, whatever its type.
    pub fn data(&self) -> &'buf [u8] {
        match self {
            GetHidReport::Input(report) => report.data(),
            GetHidReport::Feature(report) => report.data(),
        }
    }
}

/// HID report ID
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct ReportId(pub u8);

/// A single HID device that we want to present to the host.
/// This is a transport-agnostic trait that abstracts over the details of how we get reports to/from the host,
/// so that we can implement it once and then use it for both HID-I2C and HID-I3C (and potentially HID-SPI in the
/// future if we want to add support for that).
///
/// A note on error handling - the HID spec only really has one way for a device to communicate failure, and that's
/// by doing a device-initiated reset.  If any of these functions fail, a device-initiated reset will be signalled.
///
/// If you're part of an aggregate device created by impl_odp_hid_aggregate_device!, ***this will reset your peers too***.
/// Therefore, you should be very certain this is the behavior you want before you return an error from any of these functions.
///
/// The normal pattern in HID seems to be to either embed an error code in an input report or to drop the message entirely.
///
pub trait HidDevice {
    /// The maximum size of an input report (device -> host) that this device can use, expressed in bytes.
    /// This must agree with the descriptor returned by `report_descriptor()`.
    type InputReportMaxSize: ArrayLength;

    /// The maximum size of an output report (host -> device) that this device can use, expressed in bytes.
    /// This must agree with the descriptor returned by `report_descriptor()`.
    type OutputReportMaxSize: ArrayLength;

    /// The maximum size of a feature report (bidirectional) that this device can use, expressed in bytes.
    /// This must agree with the descriptor returned by `report_descriptor()`.
    type FeatureReportMaxSize: ArrayLength;

    /// The maximum number of individual report IDs that the device will have.  In most cases, this should be exactly
    /// the number of individual report IDs that the device has, but in the passthrough case where that knowledge isn't
    /// available at compile time, this will be an upper bound.  This must agree with the descriptor returned by report_descriptor().
    ///
    /// Note that this is the maximum number of unique report *IDs* - if you have a device that shares the same report ID for
    /// an input and output report, that only counts as 1.
    const MAX_REPORT_COUNT: u8;

    /// Returns the HID descriptor for this device. This isn't allowed to change, but the passthrough case means that
    /// we can't require that it be known at compile time.
    /// If the descriptor disagrees with the sizes implied by `InputReport` / `FeatureReport` / `OutputReport` / `MAX_REPORT_COUNT`, callers should not use the object.
    fn report_descriptor(&self) -> &HidReportDescriptor<'_>;

    /// Respond to an explicit request for a particular report from the host.
    ///
    /// This invokes `process_report` with the requested [`GetHidReport`].
    ///
    /// The value returned by `process_report` must be propagated back to the caller. Returning
    /// `Err(HidError)` (before `process_report` is invoked) signals that the requested report could
    /// not be produced.
    fn process_get_report<R>(
        &mut self,
        report_type: GetHidReportType,
        report_id: ReportId,
        process_report: impl AsyncFnOnce(GetHidReport<'_>) -> R,
    ) -> impl core::future::Future<Output = Result<R, HidError>>;

    /// Respond to a command from the host to handle a particular output/feature report.
    fn set_report(&mut self, report: &SetHidReport<'_>) -> impl core::future::Future<Output = Result<(), HidError>>;

    /// Blocks until the device is ready to yield an unsolicited input report.
    /// When this returns, the next call to process_next_input_report should be able to run without blocking on I/O.
    fn wait_for_input_report(&mut self) -> impl core::future::Future<Output = ()>;

    /// Returns true if there is a pending input report that can be retrieved immediately with process_next_input_report().
    /// If this returns true, it implies that wait_for_input_report() and process_next_input_report() should return immediately.
    fn has_pending_input_report(&mut self) -> bool;

    /// Process the next unsolicited input report to the transport.
    ///
    /// This blocks until an unsolicited report is available, then invokes `process_report` with a [`HidReport`].
    ///
    /// The value returned by `process_report` must be propagated back to the caller. Returning `Err(HidError)`
    /// (before `process_report` is invoked) signals an inability to retrieve a message.
    ///
    /// This is for 'unsolicited' reports that the device has decided to signal the host to retrieve.
    ///
    fn process_next_input_report<R>(
        &mut self,
        process_report: impl AsyncFnOnce(HidReport<'_>) -> R,
    ) -> impl core::future::Future<Output = Result<R, HidError>>;

    /// Called when the host commands a particular power state.
    fn set_power_state(
        &mut self,
        state: HidDevicePowerState,
    ) -> impl core::future::Future<Output = Result<(), HidError>>;

    /// Called when the device should reset its state.  The semantics of reset are device-specific, but
    /// should generally result in clearing any pending reports and returning to a known-good state.
    /// This can be called under the following circumstances:
    ///   1. The host commands a reset, which happens once at startup and can happen again at any time
    ///   2. The implementor of this trait returned HidError::TriggerReset from one of its functions, thereby requesting a reset
    ///   3. A peer HidDevice in an aggregate device triggers a device-initiated reset (see impl_odp_hid_aggregate_device! for details)
    ///
    fn reset(&mut self) -> impl core::future::Future<Output = ()>;
}

/// A HID report descriptor
pub struct HidReportDescriptor<'buf> {
    bytes: &'buf [u8],

    report_ids_implicit: bool,
}

struct HidReportDescriptorElementHeader(u8);

#[derive(Clone, Copy, Debug, PartialEq, Eq, num_enum::IntoPrimitive, num_enum::TryFromPrimitive)]
#[repr(u8)]
enum HidItemType {
    Main = 0,
    Global = 1,
    Local = 2,
    Reserved = 3,
}

impl HidReportDescriptorElementHeader {
    /// The size of this item in bytes.
    fn item_size(&self) -> usize {
        #[allow(clippy::panic)] // This is temporary we get the HID support library implemented
        if self.0 == 0b11111110 {
            panic!("Long items are not yet supported"); // TODO implement this once we have the HID support library implemented - see 6.2.2.3 of https://www.usb.org/sites/default/files/hid1_11.pdf
        }

        match self.0 & 0b11 {
            0 => 0,
            1 => 1,
            2 => 2,
            _ => 4, // per hid spec, size=3 means 4 bytes, not 3 bytes. See section 6.2.2.2 of https://www.usb.org/sites/default/files/hid1_11.pdf
        }
    }

    /// The type of the item, which is one of Main, Global, Local, or Reserved.
    fn item_type(&self) -> HidItemType {
        // This can't actually panic because we mask to 2 bits, but there's no way to express that in the type system
        #[allow(clippy::expect_used)]
        HidItemType::try_from_primitive((self.0 >> 2) & 0b11)
            .expect("HidItemType::try_from_primitive should never fail because we mask to 2 bits")
    }

    /// The tag of this item, which is a 4-bit value that identifies the specific item within its type (e.g. start collection, end collection, input, output, etc)
    fn item_tag(&self) -> u8 {
        self.0 >> 4
    }
}

impl<'buf> HidReportDescriptor<'buf> {
    /// Constructs a HID descriptor from a byte slice.
    pub fn new(bytes: &'buf [u8]) -> Result<Self, core::convert::Infallible> {
        // TODO - validation is incomplete here. When we implement the HID support library / aggregation macro, we should have more tools to validate the descriptor here.
        let mut iter = bytes.iter();
        let mut implicit = true;
        while let Some(header_bytes) = iter.next() {
            const REPORT_ID_ITEM_TAG: u8 = 0b1000; // per section 6.2.2.7
            let header = HidReportDescriptorElementHeader(*header_bytes);
            if header.item_type() == HidItemType::Global && header.item_tag() == REPORT_ID_ITEM_TAG {
                implicit = false;
                break;
            }

            if header.item_size() != 0 {
                iter.nth(header.item_size() - 1); // skip over the data bytes for this item
            }
        }

        Ok(Self {
            bytes,
            report_ids_implicit: implicit,
        })
    }

    /// Returns the raw bytes of the HID report descriptor. This is what will be sent to the host when it requests the HID descriptor.
    pub fn as_bytes(&self) -> &'buf [u8] {
        self.bytes
    }

    /// Whether or not the report IDs are implicit in the report descriptor. If true, the report ID is not sent to the host as part of the report header.
    /// This is only possible on devices that have no more than one report of each type (input, output, feature).
    pub fn report_ids_implicit(&self) -> bool {
        self.report_ids_implicit
    }
}
