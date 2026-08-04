//! HID relay code

use generic_array::ArrayLength;
use num_enum::TryFromPrimitive;

/// Errors that a HID device operation can fail with.
///
/// Reporting failure triggers a device-initiated reset, so callers must handle these errors explicitly
/// rather than swallowing them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[non_exhaustive]
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

/// A HID report
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

    /// The report ID for this report, whatever its type.
    pub fn id(&self) -> ReportId {
        match self {
            SetHidReport::Output(report) => report.id(),
            SetHidReport::Feature(report) => report.id(),
        }
    }

    /// Produce a copy of this report with a different report ID but the same data and type.
    ///
    /// This is used by aggregate [`HidDevice`]s to translate a host-facing report ID back to a
    /// sub-device's native report ID before forwarding the report.
    pub fn relabel(&self, id: ReportId) -> SetHidReport<'buf> {
        match self {
            SetHidReport::Output(report) => SetHidReport::Output(HidReport::new(id, report.data())),
            SetHidReport::Feature(report) => SetHidReport::Feature(HidReport::new(id, report.data())),
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
/// If you're part of an aggregate device created by impl_hid_aggregate_device!, ***this will reset your peers too***.
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

    /// An upper bound, in bytes, on the length of the descriptor returned by `report_descriptor()`.
    /// In the non-passthrough case this is usually the exact descriptor length; in the passthrough
    /// case (where the descriptor isn't known at compile time) it must be a safe upper bound. This
    /// lets aggregate devices allocate storage for the combined descriptor at compile time.
    const MAX_DESCRIPTOR_LEN: usize;

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
    /// Must be cancellation-safe.
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
    ///   3. A peer HidDevice in an aggregate device triggers a device-initiated reset (see impl_hid_aggregate_device! for details)
    ///
    fn reset(&mut self) -> impl core::future::Future<Output = ()>;
}

/// A HID report descriptor
///
#[derive(Debug, Clone, Copy)]
pub struct HidReportDescriptor<'a> {
    bytes: &'a [u8],

    /// true if the report descriptor uses implicit report IDs (i.e. it contains no Report ID items)
    report_ids_implicit: bool,

    /// The maximum payload size, in bytes, of each report type
    max_report_sizes: MaxReportSizes,
}

impl<'a> HidReportDescriptor<'a> {
    /// Constructs a HID descriptor from a byte slice.
    ///
    /// Returns [`HidDescriptorError`] if `bytes` is malformed, including
    /// [`HidDescriptorError::PushPopStackOverflow`] if the descriptor nests `Push` items more than
    /// 16 levels deep. Nesting that deep is uncommon - most reports only have one or two levels.
    pub fn new(bytes: &'a [u8]) -> Result<Self, HidDescriptorError> {
        /// Global-item state that persists across items and is saved/restored by `Push`/`Pop`.
        #[derive(Clone, Copy, Default)]
        struct GlobalItemState {
            report_size_bits: u32,
            report_count: u32,
            report_id: u8,
        }

        /// Accumulated payload bits for each report type.
        #[derive(Clone, Copy, Default)]
        struct ReportBits {
            input: u32,
            output: u32,
            feature: u32,
        }

        let mut report_ids_implicit = true;

        let mut current = ReportBits::default();
        let mut max = ReportBits::default();
        let mut flush = |current: &mut ReportBits| {
            max.input = max.input.max(current.input);
            max.output = max.output.max(current.output);
            max.feature = max.feature.max(current.feature);
            *current = ReportBits::default();
        };

        let mut state = GlobalItemState::default();
        let mut stack = heapless::Vec::<GlobalItemState, 16>::new();

        for item in DescriptorItems::new(bytes) {
            let item = item?;
            if item.header.is_long_item() {
                // Long items don't affect report sizing.
                continue;
            }

            match item.header.item_type() {
                HidItemType::Global => {
                    let tag = item.header.item_tag();
                    if tag == GlobalItemTag::ReportSize as u8 {
                        state.report_size_bits = item.try_value_as_int().ok_or(HidDescriptorError::TruncatedItem)?;
                    } else if tag == GlobalItemTag::ReportCount as u8 {
                        state.report_count = item.try_value_as_int().ok_or(HidDescriptorError::TruncatedItem)?;
                    } else if tag == GlobalItemTag::ReportId as u8 {
                        let &[id] = item.data else {
                            return Err(HidDescriptorError::UnsupportedReportId);
                        };
                        report_ids_implicit = false;
                        // Switching report IDs ends the current report and starts a new one.
                        if id != state.report_id {
                            flush(&mut current);
                        }
                        state.report_id = id;
                    } else if tag == GlobalItemTag::Push as u8 {
                        stack
                            .push(state)
                            .map_err(|_| HidDescriptorError::PushPopStackOverflow)?;
                    } else if tag == GlobalItemTag::Pop as u8 {
                        let restored = stack.pop().ok_or(HidDescriptorError::UnbalancedPop)?;
                        // A Pop that restores a different report ID ends the current report.
                        if restored.report_id != state.report_id {
                            flush(&mut current);
                        }
                        state = restored;
                    }
                }
                HidItemType::Main => {
                    let bits = state.report_size_bits.saturating_mul(state.report_count);
                    let field = match MainItemTag::try_from_primitive(item.header.item_tag()) {
                        Ok(MainItemTag::Input) => Some(&mut current.input),
                        Ok(MainItemTag::Output) => Some(&mut current.output),
                        Ok(MainItemTag::Feature) => Some(&mut current.feature),
                        // Collection / End Collection and reserved tags carry no report data.
                        Err(_) => None,
                    };
                    if let Some(field) = field {
                        *field = field.saturating_add(bits);
                    }
                }
                HidItemType::Local | HidItemType::Reserved => {}
            }
        }
        flush(&mut current);

        Ok(Self {
            bytes,
            report_ids_implicit,
            max_report_sizes: MaxReportSizes {
                input: bits_to_bytes(max.input),
                output: bits_to_bytes(max.output),
                feature: bits_to_bytes(max.feature),
            },
        })
    }

    /// Returns the raw bytes of the HID report descriptor. This is what will be sent to the host when it requests the HID descriptor.
    pub fn as_bytes(&self) -> &[u8] {
        self.bytes
    }

    /// True if the report descriptor uses implicit report IDs (i.e. it contains no Report ID items).
    pub fn report_ids_implicit(&self) -> bool {
        self.report_ids_implicit
    }

    /// Returns the maximum payload size, in bytes, of each report type (`Input`/`Output`/`Feature`)
    /// declared by this descriptor, as computed at construction. Sizes exclude the report ID byte.
    pub fn max_report_sizes(&self) -> MaxReportSizes {
        self.max_report_sizes
    }
}

/// The maximum payload size, in bytes, of each report type declared by a HID report descriptor.
/// Sizes exclude the report ID byte.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct MaxReportSizes {
    /// Largest input report payload, in bytes.
    pub input: usize,
    /// Largest output report payload, in bytes.
    pub output: usize,
    /// Largest feature report payload, in bytes.
    pub feature: usize,
}

/// Converts a bit count to whole bytes, rounding up.
fn bits_to_bytes(bits: u32) -> usize {
    bits.div_ceil(8) as usize
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, num_enum::TryFromPrimitive)]
#[repr(u8)]
enum HidItemType {
    Main = 0,
    Global = 1,
    Local = 2,
    Reserved = 3,
}

/// The Global item tags this code recognizes or emits (section 6.2.2.7 of the HID 1.11 spec).
///
/// Tags are only meaningful relative to the item type; these are the Global-type tags.
#[derive(Clone, Copy, Debug, PartialEq, Eq, num_enum::TryFromPrimitive)]
#[repr(u8)]
enum GlobalItemTag {
    ReportSize = 0b0111,
    ReportId = 0b1000,
    ReportCount = 0b1001,
    Push = 0b1010,
    Pop = 0b1011,
}

/// The Main item tags that declare report data fields (section 6.2.2.4 of the HID 1.11 spec). Other
/// Main tags (Collection, End Collection) carry no report payload and are intentionally omitted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, num_enum::TryFromPrimitive)]
#[repr(u8)]
enum MainItemTag {
    Input = 0b1000,
    Output = 0b1001,
    Feature = 0b1011,
}

/// The size field of a short item header
#[derive(Clone, Copy, Debug, PartialEq, Eq, num_enum::TryFromPrimitive)]
#[repr(u8)]
enum ShortItemSize {
    Zero = 0,
    One = 1,
    Two = 2,
    // note - per HID spec, a size value of 3 does mean 4 bytes
    Four = 3,
}

impl ShortItemSize {
    /// The number of data bytes this size represents.
    const fn data_bytes(self) -> usize {
        match self {
            ShortItemSize::Zero => 0,
            ShortItemSize::One => 1,
            ShortItemSize::Two => 2,
            ShortItemSize::Four => 4,
        }
    }
}

struct HidReportDescriptorElementHeader(u8);
impl HidReportDescriptorElementHeader {
    /// Encode a Global short-item header with the given `tag` and data size (section 6.2.2.2 of the
    /// HID 1.11 spec).
    const fn global(tag: GlobalItemTag, size: ShortItemSize) -> Self {
        Self(((tag as u8) << 4) | ((HidItemType::Global as u8) << 2) | (size as u8))
    }

    /// The raw header byte, for writing this item into a descriptor byte stream.
    const fn byte(&self) -> u8 {
        self.0
    }

    /// The type of the item, which is one of Main, Global, Local, or Reserved.
    fn item_type(&self) -> HidItemType {
        // Panic safety: This can't actually panic because we mask to 2 bits and the enum covers all 2-bit values, but there's no way to express that in the type system
        #[allow(clippy::expect_used)]
        HidItemType::try_from_primitive((self.0 >> 2) & 0b11)
            .expect("HidItemType::try_from_primitive should never fail because we mask to 2 bits")
    }

    /// The tag of this item, which is a 4-bit value that identifies the specific item within its type (e.g. start collection, end collection, input, output, etc)
    fn item_tag(&self) -> u8 {
        self.0 >> 4
    }

    /// Whether this is a "long item" header (see section 6.2.2.3 of the HID 1.11 spec). A long item
    /// is encoded as `[0b1111_1110, bDataSize, bLongItemTag, data..]`, i.e. a 3-byte header followed by
    /// `bDataSize` data bytes.
    fn is_long_item(&self) -> bool {
        self.0 == LONG_ITEM_HEADER
    }

    /// The number of data bytes encoded by a *short* item header. Only meaningful for short items.
    fn short_item_data_size(&self) -> ShortItemSize {
        // Panic safety: This can't actually panic because we mask to 2 bits and the enum covers all 2-bit values, but there's no way to express that in the type system
        #[allow(clippy::expect_used)]
        ShortItemSize::try_from_primitive(self.0 & 0b11).expect(
            "HidReportDescriptorElementHeader::short_item_data_size should never fail because we mask to 2 bits",
        )
    }

    /// True if this header is a Global "Report ID" item (section 6.2.2.7).
    fn is_report_id(&self) -> bool {
        self.item_type() == HidItemType::Global && self.item_tag() == GlobalItemTag::ReportId as u8
    }

    /// True if this header is a Main "Collection" item (section 6.2.2.4). The first Collection item
    /// in a descriptor opens its top-level collection.
    fn is_collection(&self) -> bool {
        self.item_type() == HidItemType::Main && self.item_tag() == COLLECTION_ITEM_TAG
    }
}

/// Main item tag for a Collection item (section 6.2.2.4). Not part of [`MainItemTag`], which only
/// covers the report-data Main items.
const COLLECTION_ITEM_TAG: u8 = 0b1010;

/// Header byte value that introduces a long item (section 6.2.2.3 of the HID 1.11 spec).
const LONG_ITEM_HEADER: u8 = 0b1111_1110;

/// Encoded header for a one-byte Global "Report ID" item (size=1).
const REPORT_ID_HEADER_SIZE1: HidReportDescriptorElementHeader =
    HidReportDescriptorElementHeader::global(GlobalItemTag::ReportId, ShortItemSize::One);

/// Encoded header for a Global "Push" item (no data).
const PUSH_HEADER: HidReportDescriptorElementHeader =
    HidReportDescriptorElementHeader::global(GlobalItemTag::Push, ShortItemSize::Zero);

/// Encoded header for a Global "Pop" item (no data).
const POP_HEADER: HidReportDescriptorElementHeader =
    HidReportDescriptorElementHeader::global(GlobalItemTag::Pop, ShortItemSize::Zero);

/// Errors that can occur while parsing or combining HID report descriptors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum HidDescriptorError {
    /// The descriptor ended in the middle of an item (a header claimed more data bytes than remained).
    TruncatedItem,

    /// A Report ID item did not use the expected single-byte encoding (it carried zero, two, or four
    /// data bytes). Report IDs are single-byte values, so any other encoding is treated as invalid.
    UnsupportedReportId,

    /// The combined descriptor requires more distinct report IDs than the configured capacity `N`,
    /// or more than the 255 distinct report IDs the HID specification can represent.
    TooManyReportIds,

    /// The caller-provided output buffer was too small to hold the combined descriptor; buffer should
    /// be at least `usize` bytes.
    OutputBufferTooSmall(usize),

    /// The descriptor nested `Push` global items more deeply than this parser supports.
    PushPopStackOverflow,

    /// The descriptor contained a `Pop` global item with no matching preceding `Push`.
    UnbalancedPop,
}

/// Iterator over the items of a HID report descriptor byte stream, handling both short and long items.
///
/// Yields `Err(HidDescriptorError::TruncatedItem)` (once) if an item claims more bytes than remain in
/// the stream, after which iteration stops.
struct DescriptorItems<'a> {
    bytes: &'a [u8],
    pos: usize,
    done: bool,
}

impl<'a> DescriptorItems<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            pos: 0,
            done: false,
        }
    }
}

/// A single parsed item from a HID report descriptor.
///
/// Both short items (`[header, data..]`) and long items (`[0b1111_1110, bDataSize, bLongItemTag, data..]`)
/// are represented: [`header`](Self::header) is the leading byte, [`data`](Self::data) is just the
/// item's data payload (excluding any header bytes), and [`raw`](Self::raw) is the complete item
/// including its header, so it can be copied verbatim.
struct DescriptorItem<'a> {
    header: HidReportDescriptorElementHeader,
    data: &'a [u8],
    raw: &'a [u8],
}

impl DescriptorItem<'_> {
    /// Reads a little-endian unsigned value from a short item's data payload (0..=4 bytes).
    /// If the value has no data or doesn't fit in a u32 (which isn't legal for short items),
    /// returns None.
    fn try_value_as_int(&self) -> Option<u32> {
        if self.data.is_empty() || self.data.len() > ShortItemSize::Four.data_bytes() {
            None
        } else {
            let mut value = 0u32;
            for (index, &byte) in self.data.iter().enumerate().take(4) {
                value |= (byte as u32) << (8 * index as u32);
            }
            Some(value)
        }
    }
}

impl<'a> Iterator for DescriptorItems<'a> {
    type Item = Result<DescriptorItem<'a>, HidDescriptorError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done || self.pos >= self.bytes.len() {
            return None;
        }

        let Some(&header_byte) = self.bytes.get(self.pos) else {
            self.done = true;
            return None;
        };
        let header = HidReportDescriptorElementHeader(header_byte);

        // A long item carries a 3-byte header (`[0b1111_1110, bDataSize, bLongItemTag]`) with its data size
        // in the second byte; a short item has a 1-byte header encoding its own data size.
        let (header_len, data_size) = if header.is_long_item() {
            let Some(&data_size) = self.bytes.get(self.pos + 1) else {
                self.done = true;
                return Some(Err(HidDescriptorError::TruncatedItem));
            };
            (3, data_size as usize)
        } else {
            (1, header.short_item_data_size().data_bytes())
        };

        let data_start = self.pos + header_len;
        let data_end = data_start + data_size;
        // `data` covers the payload; `raw` covers the whole item (header included). Both `get`s also
        // bounds-check the header bytes: a valid `data_start..data_end` implies `data_start <= len`,
        // so every header byte (which precedes `data_start`) is present.
        let Some(data) = self.bytes.get(data_start..data_end) else {
            self.done = true;
            return Some(Err(HidDescriptorError::TruncatedItem));
        };
        let Some(raw) = self.bytes.get(self.pos..data_end) else {
            self.done = true;
            return Some(Err(HidDescriptorError::TruncatedItem));
        };

        self.pos = data_end;
        Some(Ok(DescriptorItem { header, data, raw }))
    }
}

/// A bounds-checked, non-panicking writer over a caller-provided output buffer.
struct BoundedWriter<'buf> {
    buf: &'buf mut [u8],
    pos: usize,
}

impl<'buf> BoundedWriter<'buf> {
    fn new(buf: &'buf mut [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    fn push(&mut self, byte: u8) -> Result<(), HidDescriptorError> {
        let buflen = self.buf.len();
        let slot = self
            .buf
            .get_mut(self.pos)
            .ok_or(HidDescriptorError::OutputBufferTooSmall(buflen))?;
        *slot = byte;
        self.pos += 1;
        Ok(())
    }

    fn push_slice(&mut self, bytes: &[u8]) -> Result<(), HidDescriptorError> {
        let buflen = self.buf.len();
        let end = self.pos + bytes.len();
        let dst = self
            .buf
            .get_mut(self.pos..end)
            .ok_or(HidDescriptorError::OutputBufferTooSmall(buflen))?;
        dst.copy_from_slice(bytes);
        self.pos = end;
        Ok(())
    }
}

/// Partitions the `remaps` table into one contiguous block per owner (input/child). `starts[i]` is
/// the first `remaps` index owned by owner `i`, and `total` is the number of reports across all
/// owners — i.e. the end of the last owner's block.
///
#[derive(Debug)]
struct BlockBoundaries<const OWNERS: usize> {
    /// First `remaps` index owned by each owner, in owner order.
    starts: [u8; OWNERS],
    /// Total number of reports across all owners (the end of the last owner's block).
    total: u8,
}

impl<const OWNERS: usize> BlockBoundaries<OWNERS> {
    /// A fresh, all-zero table to be populated by [`ReportIdMapBuilder`].
    fn new() -> Self {
        Self {
            starts: [0u8; OWNERS],
            total: 0,
        }
    }

    /// Total number of reports across all owners.
    fn total(&self) -> usize {
        self.total as usize
    }

    /// The owner of the report at `report_index` — the last owner whose start is `<= report_index`
    /// — or `None` if `report_index` precedes every block (never happens for a valid index).
    fn owner_of(&self, report_index: usize) -> Option<usize> {
        self.starts.iter().rposition(|&start| (start as usize) <= report_index)
    }

    /// The half-open `remaps` range `[start, end)` owned by `owner`, or `None` if `owner` is out of
    /// range. The last owner's `end` is `total`.
    fn bounds(&self, owner: usize) -> Option<(usize, usize)> {
        let start = *self.starts.get(owner)? as usize;
        let end = self.starts.get(owner + 1).map_or(self.total as usize, |&s| s as usize);
        Some((start, end))
    }

    /// Record that `owner`'s block begins at `report_index`. Called once per owner in increasing
    /// order as the tables are built.
    fn set_start(&mut self, owner: usize, report_index: usize) -> Result<(), HidDescriptorError> {
        let slot = self.starts.get_mut(owner).ok_or(HidDescriptorError::TooManyReportIds)?;
        *slot = u8::try_from(report_index).map_err(|_| HidDescriptorError::TooManyReportIds)?;
        Ok(())
    }

    /// Record the total report count (the end of the last owner's block).
    fn set_total(&mut self, total: usize) -> Result<(), HidDescriptorError> {
        self.total = u8::try_from(total).map_err(|_| HidDescriptorError::TooManyReportIds)?;
        Ok(())
    }
}

/// Owns the two report-ID remapping tables produced by combining a set of HID report descriptors,
/// and exposes the lookups needed to route reports between the host and each constituent input.
///
/// Generic parameters:
///
/// * `REPORT_COUNT` — capacity of the native-report-ID table. Must be at least the total number of
///   report IDs across all inputs (for an aggregate device, the sum of the sub-devices'
///   [`HidDevice::MAX_REPORT_COUNT`]).
/// * `DEVICE_COUNT` — the number of constituent devices.
///
/// The combiner assigns host-facing report IDs contiguously from 1 in input order, then within an
/// input in declaration order.
///
#[derive(Debug)]
pub struct ReportIdMap<const REPORT_COUNT: usize, const DEVICE_COUNT: usize> {
    /// Native/subdevice-facing report ID for each host-facing ID, indexed by `host_id - 1`.
    /// (host_id 0 is not a valid report ID)
    remaps: [u8; REPORT_COUNT],
    /// Per-owner block boundaries into `remaps` (see `BlockBoundaries`).
    blocks: BlockBoundaries<DEVICE_COUNT>,
}

impl<const REPORT_COUNT: usize, const DEVICE_COUNT: usize> ReportIdMap<REPORT_COUNT, DEVICE_COUNT> {
    /// Combines multiple HID report descriptors into a single descriptor whose top-level collections
    /// are the concatenation of the inputs' top-level collections, written into caller-provided `out`
    /// storage, and captures the resulting report-ID remapping in the returned `ReportIdMap`.
    ///
    /// The combined device must present a globally-unique report ID for every report, so every report
    /// is renumbered, assigning report IDs contiguously starting from 1 in input order. If one of the
    /// sub-descriptors uses implicit report IDs, an explicit report ID is inserted for it (such a
    /// sub-device is addressed by native report ID 0).
    ///
    /// HID global items persist across the descriptor byte stream, so naively concatenating
    /// descriptors would let one input's leftover global state leak into the next and corrupt its
    /// reports. To prevent this, each input descriptor's items are wrapped in a `Push`/`Pop` pair
    /// (HID 1.11 section 6.2.2.7). The combined descriptor is therefore larger than the sum of its
    /// inputs by up to [`MAX_DESCRIPTOR_FRAMING_OVERHEAD`] bytes per input; the caller must size `out`
    /// to account for this.
    ///
    /// Returns the combined descriptor (which borrows `out`) alongside the map.
    ///
    pub fn combine<'buf>(
        inputs: &[&HidReportDescriptor<'_>; DEVICE_COUNT],
        out: &'buf mut [u8],
    ) -> Result<(HidReportDescriptor<'buf>, Self), HidDescriptorError> {
        let mut map = Self {
            remaps: [0u8; REPORT_COUNT],
            blocks: BlockBoundaries::new(),
        };

        let expected_output_buffer_size = inputs.iter().map(|input| input.as_bytes().len()).sum::<usize>()
            + inputs.len() * MAX_DESCRIPTOR_FRAMING_OVERHEAD;
        if out.len() < expected_output_buffer_size {
            return Err(HidDescriptorError::OutputBufferTooSmall(expected_output_buffer_size));
        }

        let mut writer = BoundedWriter::new(out);
        let mut builder = ReportIdMapBuilder::new(&mut map.remaps, &mut map.blocks);

        for (input_index, input) in inputs.iter().enumerate() {
            builder.begin_input(input_index)?;

            writer.push(PUSH_HEADER.byte())?;

            // An implicit descriptor (no report ID item) still needs a unique report ID. The report
            // ID item must be inserted *inside* a top-level collection, so defer it until entering
            // the descriptor's first top-level collection.
            let implicit = input.report_ids_implicit();
            let mut report_id_inserted = false;

            // Walk the descriptor's items once, assigning each distinct declared report ID the next
            // contiguous host-facing ID the first time it is seen and rewriting the item to carry it.
            for item in DescriptorItems::new(input.as_bytes()) {
                let item = item?;
                if item.header.is_report_id() {
                    // A report ID is a single-byte value, so a wider (or empty) Report ID item
                    // encoding is rejected as invalid input.
                    let &[old_id] = item.data else {
                        return Err(HidDescriptorError::UnsupportedReportId);
                    };
                    let new_id = builder.assign(old_id)?;
                    // Rewrite the (size-1) report ID item to carry the newly-assigned host-facing ID,
                    // preserving its header.
                    writer.push(item.header.byte())?;
                    writer.push(new_id)?;
                } else {
                    // Every other item (including long items) is copied through unchanged.
                    writer.push_slice(item.raw)?;

                    // Insert the implicit descriptor's report ID immediately inside its first
                    // (top-level) collection so the declaration is not left outside it.
                    if implicit && !report_id_inserted && item.header.is_collection() {
                        let inserted_id = builder.record(0)?;
                        writer.push(REPORT_ID_HEADER_SIZE1.byte())?;
                        writer.push(inserted_id)?;
                        report_id_inserted = true;
                    }
                }
            }

            writer.push(POP_HEADER.byte())?;
        }

        builder.finish()?;

        let len = writer.pos;
        let bytes: &'buf [u8] = writer
            .buf
            .get(..len)
            .ok_or(HidDescriptorError::OutputBufferTooSmall(expected_output_buffer_size))?;

        Ok((HidReportDescriptor::new(bytes)?, map))
    }

    /// Total number of reports (valid host-facing IDs are `1..=len()`).
    pub fn len(&self) -> usize {
        self.blocks.total()
    }

    /// Whether the map contains no reports.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The `remaps` index backing a host-facing report ID, or `None` if the ID is out of range.
    /// IDs are assigned contiguously from 1, so the index is simply `host_id - 1`.
    fn index_of(&self, host_id: u8) -> Option<usize> {
        let index = (host_id as usize).checked_sub(1)?;
        (index < self.len()).then_some(index)
    }

    /// Map a host-facing report ID to the `(owner index, native report ID)` that owns it.
    pub fn route(&self, host_id: u8) -> Option<(usize, u8)> {
        let index = self.index_of(host_id)?;
        let owner = self.blocks.owner_of(index)?;
        let native = self.remaps.get(index).copied()?;
        Some((owner, native))
    }

    /// Reverse lookup: the host-facing report ID for `native_id` within `owner`'s block, or `None`
    /// if that owner declares no such native report. Only the owner's block of `remaps` is scanned.
    pub fn host_id(&self, owner: usize, native_id: u8) -> Option<u8> {
        let (start, end) = self.blocks.bounds(owner)?;
        let offset = self.remaps.get(start..end)?.iter().position(|&id| id == native_id)?;
        u8::try_from(start + offset + 1).ok()
    }
}

/// Builds the report-ID remapping tables (`remaps` + a [`BlockBoundaries`]) that [`ReportIdMap`]
/// later reads, writing into caller-provided storage. This is the write-side counterpart and the single
/// place that knows how host IDs are assigned: contiguously from 1, in the order
/// [`record`](Self::record) is called.
struct ReportIdMapBuilder<'a, const OWNERS: usize> {
    /// Native report ID per assigned host ID, indexed by `host_id - 1`.
    remaps: &'a mut [u8],
    /// Per-owner block boundaries filled by [`begin_input`](Self::begin_input) and
    /// [`finish`](Self::finish).
    blocks: &'a mut BlockBoundaries<OWNERS>,
    /// Number of reports recorded so far. Also the next free `remaps` slot and the value the next
    /// host ID is one greater than.
    count: usize,
    /// First `remaps` index of the input currently being built (its block start), used to scope
    /// [`assigned`](Self::assigned) to the current input.
    input_start: usize,
}

impl<'a, const OWNERS: usize> ReportIdMapBuilder<'a, OWNERS> {
    fn new(remaps: &'a mut [u8], blocks: &'a mut BlockBoundaries<OWNERS>) -> Self {
        Self {
            remaps,
            blocks,
            count: 0,
            input_start: 0,
        }
    }

    /// Begin the block for input `input_index`, recording its start boundary. Inputs must be begun
    /// in increasing `input_index` order before any of that input's reports are recorded.
    fn begin_input(&mut self, input_index: usize) -> Result<(), HidDescriptorError> {
        self.input_start = self.count;
        self.blocks.set_start(input_index, self.count)
    }

    /// The host-facing ID already assigned to `native_id` within the current input's block, or
    /// `None` if it has not been recorded yet.
    fn assigned(&self, native_id: u8) -> Option<u8> {
        let offset = self
            .remaps
            .get(self.input_start..self.count)?
            .iter()
            .position(|&id| id == native_id)?;
        u8::try_from(self.input_start + offset + 1).ok()
    }

    /// Record a new report with native ID `native_id`, returning the host-facing ID it is assigned.
    /// IDs are handed out contiguously from 1, so the assigned ID is `count + 1`.
    ///
    /// Errors with [`HidDescriptorError::TooManyReportIds`] if the assigned ID would exceed the
    /// single-byte report-ID space or number of configured remaps
    fn record(&mut self, native_id: u8) -> Result<u8, HidDescriptorError> {
        let assigned = u8::try_from(self.count + 1).map_err(|_| HidDescriptorError::TooManyReportIds)?;
        let slot = self
            .remaps
            .get_mut(self.count)
            .ok_or(HidDescriptorError::TooManyReportIds)?;
        *slot = native_id;
        self.count += 1;
        Ok(assigned)
    }

    /// Return the host-facing ID for `native_id` in the current input, recording it if it is new.
    fn assign(&mut self, native_id: u8) -> Result<u8, HidDescriptorError> {
        match self.assigned(native_id) {
            Some(id) => Ok(id),
            None => self.record(native_id),
        }
    }

    /// Finalize the tables: record the total report count and return it.
    fn finish(self) -> Result<usize, HidDescriptorError> {
        self.blocks.set_total(self.count)?;
        Ok(self.count)
    }
}

/// Upper bound on the framing bytes [`ReportIdMap::combine`] adds per input descriptor. Each input
/// is wrapped in a `Push`/`Pop` pair (1 byte each) and, for implicit descriptors, gains an inserted
/// Report ID item (a 1-byte header plus a 1-byte value): 4 bytes total in the worst case.
///
/// Generally, you should use the [`impl_hid_aggregate_device!`](crate::impl_hid_aggregate_device) macro
/// rather than sizing buffers yourself, but if you need to manually implement an aggregate device with
/// report ID remapping, use this to size the buffer you pass to [`ReportIdMap::combine`].
pub const MAX_DESCRIPTOR_FRAMING_OVERHEAD: usize = 4;

/// Generates a new type that implements [`HidDevice`] by aggregating a fixed set of child
/// [`HidDevice`]s. The generated device:
///
/// - Exposes a single report descriptor built at runtime from its children's descriptors, with all
///   top-level collections concatenated in the order the children are listed and conflicting report
///   IDs renumbered to be globally unique (see [`ReportIdMap::combine`]).
///   NOTE: Because report IDs may be reassigned, children cannot rely on the host observing a
///   specific report ID. If you need that level of control, implement [`HidDevice`] by hand.
/// - Routes incoming `get_report`/`set_report` calls to the owning child based on report ID,
///   translating the host-facing report ID back to the child's native report ID.
/// - Relabels each child's unsolicited input reports from the child's native report ID to the
///   host-facing report ID.
/// - Fans `set_power_state`/`reset` out to every child.
/// - On a device-initiated reset commanded by any child, returns [`HidError::TriggerReset`],
///   which triggers a device-initiated reset of the *entire* aggregate (and therefore all children).
///
/// This is analogous to the `impl_odp_mctp_relay_handler!()` macro for MCTP.
///
/// # Usage
///
/// ```ignore
/// impl_hid_aggregate_device!(pub MyCombinedDevice: MyKeyboard, MyMouse<'static>);
///
/// let mut resources = MyCombinedDeviceResources::new();
/// let combined = MyCombinedDevice::new(&mut resources, keyboard, mouse)?;
/// ```
///
#[macro_export]
macro_rules! impl_hid_aggregate_device {
    ($vis:vis $out:ident : $($dev:ty),+ $(,)?) => {
        $crate::impl_hid_aggregate_device!(
            // TODO This 'zip' trick is a hack to get around the limitation in rust macros that
            //      they can't get indices / counts of the number of elements in a repeated list.
            //      For now, we're limited to 16 child devices; if we need to expand that, we can
            //      extend this array, but there's a hard cap of 255 reports for a single HID device,
            //      so it seems unlikely that we'd need more than 16 child devices in practice.
            //
            //      When macro_metavar_expr stabilizes, we may be able to leverage it to eliminate
            //      this limitation.
            //
            @zip $vis $out ; [] ; [0 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15] ; $($dev),+
        );
    };

    // Count the child types as a `usize` const expression via the length of a unit array.
    (@count $($dev:ty),+ $(,)?) => {
        <[()]>::len(&[ $( $crate::impl_hid_aggregate_device!(@unit $dev) ),+ ])
    };
    (@unit $dev:ty) => { () };

    // Get the maximum value of size_property from all HidDevices in the list.
    // size_property must be a typenum type.
    (@max_size $size_property:ident ; $head:ty) => {
        <$head as $crate::relay::hid::HidDevice>::$size_property
    };
    (@max_size $size_property:ident ; $head:ty, $($tail:ty),+) => {
        $crate::_macro_internal::typenum::Maximum<
            <$head as $crate::relay::hid::HidDevice>::$size_property,
            $crate::impl_hid_aggregate_device!(@max_size $size_property ; $($tail),+)
        >
    };

    // Zip each child type with a positional index drawn from the supply.
    (@zip $vis:vis $out:ident ; [$($acc:tt)*] ; [$i:tt $($irest:tt)*] ; $head:ty $(, $tail:ty)*) => {
        $crate::impl_hid_aggregate_device!(
            @zip $vis $out ; [$($acc)* $i : $head ,] ; [$($irest)*] ; $($tail),*
        );
    };

    // No more child types: emit the aggregate.
    (@zip $vis:vis $out:ident ; [$($idx:tt : $dev:ty ,)*] ; [$($unused:tt)*] ; ) => {
        $crate::impl_hid_aggregate_device!(@emit $vis $out ; $($idx : $dev),*);
    };

    (@emit $vis:vis $out:ident ; $($idx:tt : $dev:ty),+ ) => {
        $crate::_macro_internal::paste::paste! {
        // Enum for dispatch over the child devices
        #[doc(hidden)]
        enum [< $out ChildDevice >] {
            $( [< Variant $idx >]($dev), )+
        }

        impl [< $out ChildDevice >] {
            async fn process_get_report<R>(
                &mut self,
                report_type: $crate::relay::hid::GetHidReportType,
                report_id: $crate::relay::hid::ReportId,
                process_report: impl AsyncFnOnce($crate::relay::hid::GetHidReport<'_>) -> R,
            ) -> ::core::result::Result<R, $crate::relay::hid::HidError> {
                match self {
                    $( Self::[< Variant $idx >](child) => {
                        $crate::relay::hid::HidDevice::process_get_report(child, report_type, report_id, process_report).await
                    } )+
                }
            }

            async fn set_report(
                &mut self,
                report: &$crate::relay::hid::SetHidReport<'_>,
            ) -> ::core::result::Result<(), $crate::relay::hid::HidError> {
                match self {
                    $( Self::[< Variant $idx >](child) => {
                        $crate::relay::hid::HidDevice::set_report(child, report).await
                    } )+
                }
            }

            async fn wait_for_input_report(&mut self) {
                match self {
                    $( Self::[< Variant $idx >](child) => {
                        $crate::relay::hid::HidDevice::wait_for_input_report(child).await
                    } )+
                }
            }

            fn has_pending_input_report(&mut self) -> bool {
                match self {
                    $( Self::[< Variant $idx >](child) => {
                        $crate::relay::hid::HidDevice::has_pending_input_report(child)
                    } )+
                }
            }

            async fn process_next_input_report<R>(
                &mut self,
                process_report: impl AsyncFnOnce($crate::relay::hid::HidReport<'_>) -> R,
            ) -> ::core::result::Result<R, $crate::relay::hid::HidError> {
                match self {
                    $( Self::[< Variant $idx >](child) => {
                        $crate::relay::hid::HidDevice::process_next_input_report(child, process_report).await
                    } )+
                }
            }

            async fn set_power_state(
                &mut self,
                state: $crate::relay::hid::HidDevicePowerState,
            ) -> ::core::result::Result<(), $crate::relay::hid::HidError> {
                match self {
                    $( Self::[< Variant $idx >](child) => {
                        $crate::relay::hid::HidDevice::set_power_state(child, state).await
                    } )+
                }
            }

            async fn reset(&mut self) {
                match self {
                    $( Self::[< Variant $idx >](child) => {
                        $crate::relay::hid::HidDevice::reset(child).await
                    } )+
                }
            }
        }

        // Worst-case combined report descriptor length: each child's upper bound plus the per-child
        // framing overhead `combine` may add. Shared by `Resources` and `MAX_DESCRIPTOR_LEN`.
        #[doc(hidden)]
        const [< $out:snake:upper _DESCRIPTOR_BUF_LEN >]: usize =
            (0usize $(+ <$dev as $crate::relay::hid::HidDevice>::MAX_DESCRIPTOR_LEN)+)
                + $crate::relay::hid::MAX_DESCRIPTOR_FRAMING_OVERHEAD
                    * $crate::impl_hid_aggregate_device!(@count $($dev),+);

        #[doc = "Caller-allocated storage for the aggregate `HidDevice` generated by `impl_hid_aggregate_device!`."]
        $vis struct [< $out Resources >] {
            // Scratch buffer the combined report descriptor is written into and borrowed from. Sized
            // to the worst-case combined length; `combine` writes and returns only the prefix it uses.
            descriptor: [u8; [< $out:snake:upper _DESCRIPTOR_BUF_LEN >]],
        }

        impl [< $out Resources >] {
            /// Create zeroed storage for the aggregate's combined report descriptor.
            $vis const fn new() -> Self {
                Self {
                    descriptor: [0u8; [< $out:snake:upper _DESCRIPTOR_BUF_LEN >]],
                }
            }
        }

        impl ::core::default::Default for [< $out Resources >] {
            fn default() -> Self {
                Self::new()
            }
        }

        #[doc = "Aggregate `HidDevice` generated by `impl_hid_aggregate_device!`."]
        $vis struct $out<'a> {
            // Children stored in a flat array of the homogeneous `ChildDevice` enum, indexed by the
            // owner index the report-ID map resolves to.
            children: [
                [< $out ChildDevice >];
                $crate::impl_hid_aggregate_device!(@count $($dev),+)
            ],
            // A report descriptor that combines all the child report descriptors
            descriptor: $crate::relay::hid::HidReportDescriptor<'a>,
            // Report-ID remapping table
            map: $crate::relay::hid::ReportIdMap<
                { 0usize $(+ <$dev as $crate::relay::hid::HidDevice>::MAX_REPORT_COUNT as usize)+ },
                { $crate::impl_hid_aggregate_device!(@count $($dev),+) },
            >,
        }

        impl<'a> $out<'a> {
            /// Construct the aggregate device.
            ///
            /// Combines the children's report descriptors (renumbering all report IDs contiguously from 1)
            /// and creates an instance of the aggregate device, which handles translation of report IDs
            /// between the host-facing IDs and the child-declared IDs.
            /// The child devices are passed as individual arguments in the same order they
            /// were listed in the macro invocation.
            $vis fn new(
                resources: &'a mut [< $out Resources >],
                $( [< child $idx >]: $dev, )+
            ) -> ::core::result::Result<Self, $crate::relay::hid::HidDescriptorError> {
                let (descriptor, map) = $crate::relay::hid::ReportIdMap::combine(
                    &[ $( $crate::relay::hid::HidDevice::report_descriptor(& [< child $idx >]) ),+ ],
                    &mut resources.descriptor,
                )?;

                let children = [
                    $( [< $out ChildDevice >]::[< Variant $idx >]([< child $idx >]) ),+
                ];
                ::core::result::Result::Ok(Self {
                    children,
                    descriptor,
                    map,
                })
            }
        }

        impl<'a> $crate::relay::hid::HidDevice for $out<'a> {
            type InputReportMaxSize =
                $crate::impl_hid_aggregate_device!(@max_size InputReportMaxSize ; $($dev),+);
            type OutputReportMaxSize =
                $crate::impl_hid_aggregate_device!(@max_size OutputReportMaxSize ; $($dev),+);
            type FeatureReportMaxSize =
                $crate::impl_hid_aggregate_device!(@max_size FeatureReportMaxSize ; $($dev),+);
            const MAX_REPORT_COUNT: u8 =
                0u8 $(+ <$dev as $crate::relay::hid::HidDevice>::MAX_REPORT_COUNT)+;
            const MAX_DESCRIPTOR_LEN: usize = [< $out:snake:upper _DESCRIPTOR_BUF_LEN >];

            fn report_descriptor(&self) -> &$crate::relay::hid::HidReportDescriptor<'_> {
                &self.descriptor
            }

            async fn process_get_report<R>(
                &mut self,
                report_type: $crate::relay::hid::GetHidReportType,
                report_id: $crate::relay::hid::ReportId,
                process_report: impl AsyncFnOnce($crate::relay::hid::GetHidReport<'_>) -> R,
            ) -> ::core::result::Result<R, $crate::relay::hid::HidError> {
                match self.map.route(report_id.0) {
                    ::core::option::Option::Some((index, native)) => {
                        match self.children.get_mut(index) {
                            ::core::option::Option::Some(child) => {
                                child.process_get_report(
                                    report_type,
                                    $crate::relay::hid::ReportId(native),
                                    process_report,
                                )
                                .await
                            }
                            // Should be unreachable; `route` only yields owner indices in `0..children.len()`.
                            ::core::option::Option::None => {
                                ::core::result::Result::Err($crate::relay::hid::HidError::TriggerReset)
                            }
                        }
                    }
                    ::core::option::Option::None => {
                        ::core::result::Result::Err($crate::relay::hid::HidError::TriggerReset)
                    }
                }
            }

            async fn set_report(
                &mut self,
                report: &$crate::relay::hid::SetHidReport<'_>,
            ) -> ::core::result::Result<(), $crate::relay::hid::HidError> {
                match self.map.route(report.id().0) {
                    ::core::option::Option::Some((index, native)) => {
                        let relabelled = report.relabel($crate::relay::hid::ReportId(native));
                        match self.children.get_mut(index) {
                            ::core::option::Option::Some(child) => {
                                child.set_report(&relabelled).await
                            }
                            // Should be unreachable (see `process_get_report`);
                            ::core::option::Option::None => {
                                ::core::result::Result::Err($crate::relay::hid::HidError::TriggerReset)
                            }
                        }
                    }
                    ::core::option::Option::None => {
                        ::core::result::Result::Err($crate::relay::hid::HidError::TriggerReset)
                    }
                }
            }

            async fn wait_for_input_report(&mut self) {
                // Drop safety: `wait_for_input_report` only peeks readiness and returns nothing, so
                // dropping the futures for the children that didn't win the race cannot lose a
                // pending report — the winning child's report stays queued until it is drained by
                // `process_next_input_report`.
                let _ = $crate::_macro_internal::embassy_futures::select::select_array(
                    self.children.each_mut().map(|child| child.wait_for_input_report()),
                )
                .await;
            }

            fn has_pending_input_report(&mut self) -> bool {
                self.children.iter_mut().any(|child| child.has_pending_input_report())
            }

           async fn process_next_input_report<R>(
                &mut self,
                process_report: impl AsyncFnOnce($crate::relay::hid::HidReport<'_>) -> R,
            ) -> ::core::result::Result<R, $crate::relay::hid::HidError> {
                // Wait until some child has a report, then note which one. This lets us avoid
                // drop-safety problems that might occur if we were select!ing over multiple children
                // that were ready to yield an input report at the same time
                let index = loop {
                    match self.children.iter_mut().position(|child| child.has_pending_input_report()) {
                        ::core::option::Option::Some(i) => break i,
                        ::core::option::Option::None => {
                            $crate::relay::hid::HidDevice::wait_for_input_report(&mut *self).await
                        }
                    }
                };

                // Split the borrow so the map is readable inside the callback while the children
                // are borrowed mutably.
                let Self { children, map, .. } = self;

                match children.get_mut(index) {
                    ::core::option::Option::Some(child) => {
                        child
                            .process_next_input_report(
                                async move |report: $crate::relay::hid::HidReport<'_>| {
                                    let native = report.id().0;
                                    // Relabel the child's native report ID to the host-facing ID. If
                                    // the ID isn't in this child's block (shouldn't happen), pass it
                                    // through.
                                    let host_id = map.host_id(index, native).unwrap_or(native);
                                    process_report($crate::relay::hid::HidReport::new(
                                        $crate::relay::hid::ReportId(host_id),
                                        report.data(),
                                    ))
                                    .await
                                },
                            )
                            .await
                    }
                    ::core::option::Option::None => {
                        ::core::result::Result::Err($crate::relay::hid::HidError::TriggerReset)
                    }
                }
            }

            async fn set_power_state(
                &mut self,
                state: $crate::relay::hid::HidDevicePowerState,
            ) -> ::core::result::Result<(), $crate::relay::hid::HidError>
            {
                for child in self.children.iter_mut() {
                    child.set_power_state(state).await?;
                }
                ::core::result::Result::Ok(())
            }

            async fn reset(&mut self) {
                for child in self.children.iter_mut() {
                    child.reset().await;
                }
            }
        }
        }
    };
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]
mod tests {
    use super::*;

    // A minimal, well-formed application collection with one one-byte input report and an explicit
    // Report ID. The report ID value lives at index 7. `const` so it can also initialize statics.
    const fn explicit_descriptor(report_id: u8) -> [u8; 17] {
        [
            0x05, 0x01, // Usage Page (Generic Desktop)
            0x09, 0x02, // Usage (Mouse)
            0xA1, 0x01, // Collection (Application)
            0x85, report_id, //   Report ID (report_id)
            0x09, 0x30, //   Usage (X)
            0x75, 0x08, //   Report Size (8)
            0x95, 0x01, //   Report Count (1)
            0x81, 0x02, //   Input (Data,Var,Abs)
            0xC0, // End Collection
        ]
    }

    // The same collection but with no Report ID item (implicit report ID form).
    const IMPLICIT_DESCRIPTOR: &[u8] = &[
        0x05, 0x01, // Usage Page (Generic Desktop)
        0x09, 0x02, // Usage (Mouse)
        0xA1, 0x01, // Collection (Application)
        0x09, 0x30, //   Usage (X)
        0x75, 0x08, //   Report Size (8)
        0x95, 0x01, //   Report Count (1)
        0x81, 0x02, //   Input (Data,Var,Abs)
        0xC0, // End Collection
    ];

    /// Returns all Report ID values (Global tag 8) found in a descriptor, in order.
    fn report_ids(bytes: &[u8]) -> heapless::Vec<u8, 32> {
        let mut ids = heapless::Vec::new();
        for item in DescriptorItems::new(bytes) {
            let item = item.unwrap();
            if item.header.is_report_id() {
                ids.push(item.data[0]).unwrap();
            }
        }
        ids
    }

    /// Test helper: combine into `out`, discarding the map and returning just the descriptor. Used by
    /// the tests that only care about the combined bytes, not the remapping.
    fn combine<'buf, const REPORTS: usize, const DEVICES: usize>(
        inputs: &[&HidReportDescriptor<'_>; DEVICES],
        out: &'buf mut [u8],
    ) -> Result<HidReportDescriptor<'buf>, HidDescriptorError> {
        ReportIdMap::<REPORTS, DEVICES>::combine(inputs, out).map(|(descriptor, _)| descriptor)
    }

    #[test]
    fn report_ids_are_reassigned_contiguously() {
        // Neither input uses id 1, yet the combined descriptor renumbers every report contiguously
        // from 1 in input order, so the original values are not preserved.
        let a = explicit_descriptor(5);
        let b = explicit_descriptor(8);
        let mut out = [0u8; 64];
        let combined = combine::<8, _>(
            &[
                &HidReportDescriptor::new(&a).unwrap(),
                &HidReportDescriptor::new(&b).unwrap(),
            ],
            &mut out,
        )
        .unwrap();

        // Length is the concatenation of both inputs plus a Push/Pop pair (2 bytes) per input.
        assert_eq!(combined.as_bytes().len(), a.len() + b.len() + 4);
        // Report IDs are reassigned 1, 2 regardless of the (5, 8) inputs.
        assert_eq!(&report_ids(combined.as_bytes())[..], &[1, 2]);
        // A combined descriptor never uses implicit report IDs.
        assert!(!combined.report_ids_implicit());
    }

    #[test]
    fn all_ids_reassigned_contiguously_within_and_across_descriptors() {
        // First descriptor uses id 2; second uses ids {2, 3}. Every report is renumbered contiguously
        // in declaration order across both inputs.
        let a = explicit_descriptor(2);
        let b = [
            0x05, 0x01, // Usage Page (Generic Desktop)
            0x09, 0x02, // Usage (Mouse)
            0xA1, 0x01, // Collection (Application)
            0x85, 0x02, // Report ID (2)
            0x81, 0x02, // Input
            0x85, 0x03, // Report ID (3)
            0x81, 0x02, // Input
            0xC0, // End Collection
        ];
        let mut out = [0u8; 64];
        let combined = combine::<8, _>(
            &[
                &HidReportDescriptor::new(&a).unwrap(),
                &HidReportDescriptor::new(&b).unwrap(),
            ],
            &mut out,
        )
        .unwrap();

        // a's 2 -> 1; b's 2 -> 2; b's 3 -> 3.
        assert_eq!(&report_ids(combined.as_bytes())[..], &[1, 2, 3]);
    }

    #[test]
    fn implicit_descriptor_gets_report_id_inserted() {
        let a = IMPLICIT_DESCRIPTOR;
        let b = explicit_descriptor(1);
        let mut out = [0u8; 64];
        let combined = combine::<8, _>(
            &[
                &HidReportDescriptor::new(a).unwrap(),
                &HidReportDescriptor::new(&b).unwrap(),
            ],
            &mut out,
        )
        .unwrap();

        // Implicit descriptor gets id 1 inserted; explicit descriptor's conflicting 1 -> 2.
        assert_eq!(&report_ids(combined.as_bytes())[..], &[1, 2]);
        // Length grew by the inserted 2-byte Report ID item plus a Push/Pop pair per input.
        assert_eq!(combined.as_bytes().len(), a.len() + b.len() + 2 + 4);
        // Each input is wrapped in Push/Pop. The inserted Report ID item must land *inside* the
        // top-level collection, i.e. right after the Collection (Application) item (0xA1, 0x01),
        // which sits at index 5-6 (Push, then Usage Page + Usage from the descriptor).
        assert_eq!(combined.as_bytes()[0], PUSH_HEADER.byte());
        assert_eq!(&combined.as_bytes()[5..7], &[0xA1, 0x01]);
        assert_eq!(combined.as_bytes()[7], REPORT_ID_HEADER_SIZE1.byte());
        assert_eq!(combined.as_bytes()[8], 1);
    }

    #[test]
    fn every_report_id_is_inside_a_collection() {
        // Windows fails enumeration if any Report ID item appears outside a collection. Verify that
        // every Report ID in a combined descriptor, including the one inserted for the implicit input
        // is emitted while a collection is open.
        const END_COLLECTION_HEADER: u8 = 0xC0; // Main / End Collection, no data
        let mut out = [0u8; 64];
        let combined = combine::<8, _>(
            &[
                &HidReportDescriptor::new(IMPLICIT_DESCRIPTOR).unwrap(),
                &HidReportDescriptor::new(&explicit_descriptor(1)).unwrap(),
            ],
            &mut out,
        )
        .unwrap();

        let mut depth = 0i32;
        let mut report_id_count = 0;
        for item in DescriptorItems::new(combined.as_bytes()) {
            let item = item.unwrap();
            if item.header.is_report_id() {
                report_id_count += 1;
                assert!(depth > 0, "Report ID declared outside of a collection");
            }
            if item.header.is_collection() {
                depth += 1;
            } else if item.header.byte() == END_COLLECTION_HEADER {
                depth -= 1;
            }
        }
        assert_eq!(
            report_id_count, 2,
            "combined descriptor did not contain both report IDs"
        );
        assert_eq!(depth, 0, "unbalanced collections in combined descriptor");
    }

    #[test]
    fn each_descriptor_is_wrapped_in_push_pop() {
        let a = explicit_descriptor(1);
        let b = explicit_descriptor(2);
        let mut out = [0u8; 64];
        let combined = combine::<8, _>(
            &[
                &HidReportDescriptor::new(&a).unwrap(),
                &HidReportDescriptor::new(&b).unwrap(),
            ],
            &mut out,
        )
        .unwrap();
        let bytes = combined.as_bytes();

        // Exactly one Push and one Pop per input, balanced.
        let pushes = bytes.iter().filter(|&&b| b == PUSH_HEADER.byte()).count();
        let pops = bytes.iter().filter(|&&b| b == POP_HEADER.byte()).count();
        assert_eq!(pushes, 2);
        assert_eq!(pops, 2);
        // Starts with a Push and ends with a Pop, and the boundary between the two inputs is Pop then
        // Push (so the second input starts from a restored global state).
        assert_eq!(bytes.first(), Some(&PUSH_HEADER.byte()));
        assert_eq!(bytes.last(), Some(&POP_HEADER.byte()));
        assert!(bytes.windows(2).any(|w| w == [POP_HEADER.byte(), PUSH_HEADER.byte()]));
    }

    #[test]
    fn multiple_implicit_descriptors_get_distinct_ids() {
        let mut out = [0u8; 64];
        let combined = combine::<8, _>(
            &[
                &HidReportDescriptor::new(IMPLICIT_DESCRIPTOR).unwrap(),
                &HidReportDescriptor::new(IMPLICIT_DESCRIPTOR).unwrap(),
            ],
            &mut out,
        )
        .unwrap();

        assert_eq!(&report_ids(combined.as_bytes())[..], &[1, 2]);
    }

    #[test]
    fn output_buffer_too_small_errors() {
        let a = explicit_descriptor(1);
        let mut out = [0u8; 4];
        let err = combine::<8, _>(&[&HidReportDescriptor::new(&a).unwrap()], &mut out).unwrap_err();
        assert_eq!(err, HidDescriptorError::OutputBufferTooSmall);
    }

    #[test]
    fn long_item_is_preserved() {
        // A descriptor containing a long item (`[0b1111_1110, bDataSize, bLongItemTag, data..]`) alongside a
        // Report ID. The long item must be parsed and copied through the combiner verbatim.
        const LONG_ITEM: &[u8] = &[LONG_ITEM_HEADER, 0x02, 0x42, 0xAA, 0xBB];
        #[rustfmt::skip]
        let bytes = [
            0x06, 0x00, 0xFF,                         // Usage Page (vendor-defined)
            0x85, 0x03,                               //   Report ID (3)
            LONG_ITEM_HEADER, 0x02, 0x42, 0xAA, 0xBB, //   Long item: 2 data bytes, tag 0x42
            0x81, 0x02,                               //   Input (Data,Var,Abs)
        ];

        // The iterator splits the 3-byte long header from the payload and exposes the whole item.
        let long = DescriptorItems::new(&bytes).nth(2).unwrap().unwrap();
        assert!(long.header.is_long_item());
        assert_eq!(long.data, &[0xAA, 0xBB]);
        assert_eq!(long.raw, LONG_ITEM);

        // Combining a single input renumbers its Report ID to 1 but leaves the long item untouched.
        let mut out = [0u8; 64];
        let combined = combine::<8, _>(&[&HidReportDescriptor::new(&bytes).unwrap()], &mut out).unwrap();
        assert_eq!(&report_ids(combined.as_bytes())[..], &[1]);
        // The long item survives verbatim in the combined output.
        assert!(combined.as_bytes().windows(LONG_ITEM.len()).any(|w| w == LONG_ITEM));
    }

    #[test]
    fn truncated_long_item_errors() {
        // `bDataSize` promises two data bytes but only one is present, so parsing the descriptor
        // surfaces the truncation rather than silently swallowing it.
        let bytes = [LONG_ITEM_HEADER, 0x02, 0x42, 0xAA];
        let err = HidReportDescriptor::new(&bytes).unwrap_err();
        assert_eq!(err, HidDescriptorError::TruncatedItem);
    }

    #[test]
    fn remaps_report_conflict_resolution() {
        // Two sub-devices that both natively use report ID 1
        let a = explicit_descriptor(1);
        let b = explicit_descriptor(1);
        let mut out = [0u8; 64];
        let (combined, map) = ReportIdMap::<8, 2>::combine(
            &[
                &HidReportDescriptor::new(&a).unwrap(),
                &HidReportDescriptor::new(&b).unwrap(),
            ],
            &mut out,
        )
        .unwrap();

        assert_eq!(map.len(), 2);
        // Both inputs' native ID is 1; the conflict is resolved by handing out host IDs 1 and 2.
        assert_eq!(map.route(1), Some((0, 1))); // host 1 -> input 0's native 1
        assert_eq!(map.route(2), Some((1, 1))); // host 2 -> input 1's native 1
        // Reverse lookups round-trip within each child's block.
        assert_eq!(map.host_id(0, 1), Some(1));
        assert_eq!(map.host_id(1, 1), Some(2));
        assert_eq!(map.route(3), None);
        assert_eq!(&report_ids(combined.as_bytes())[..], &[1, 2]);
    }

    #[test]
    fn remaps_record_implicit_as_original_zero() {
        let mut out = [0u8; 64];
        let (_, map) =
            ReportIdMap::<4, 1>::combine(&[&HidReportDescriptor::new(IMPLICIT_DESCRIPTOR).unwrap()], &mut out).unwrap();

        assert_eq!(map.len(), 1);
        // An implicit sub-device is addressed by native report ID 0 and gets assigned host ID 1.
        assert_eq!(map.route(1), Some((0, 0)));
        assert_eq!(map.host_id(0, 0), Some(1));
    }

    #[test]
    fn remap_buffer_too_small_errors() {
        let a = explicit_descriptor(1);
        let b = explicit_descriptor(1);
        let mut out = [0u8; 64];
        // REPORT_COUNT = 1 cannot hold both reports.
        let err = ReportIdMap::<1, 2>::combine(
            &[
                &HidReportDescriptor::new(&a).unwrap(),
                &HidReportDescriptor::new(&b).unwrap(),
            ],
            &mut out,
        )
        .unwrap_err();
        assert_eq!(err, HidDescriptorError::TooManyReportIds);
    }

    // ---- max_report_sizes ----

    #[test]
    fn max_report_sizes_single_input() {
        // explicit_descriptor declares one 8-bit input field (Report Size 8 x Report Count 1).
        let a = explicit_descriptor(5);
        let sizes = HidReportDescriptor::new(&a).unwrap().max_report_sizes();
        assert_eq!(
            sizes,
            MaxReportSizes {
                input: 1,
                output: 0,
                feature: 0
            }
        );
    }

    #[test]
    fn max_report_sizes_implicit_report_id() {
        // No Report ID item still accumulates into the implicit (0) report slot.
        let sizes = HidReportDescriptor::new(IMPLICIT_DESCRIPTOR)
            .unwrap()
            .max_report_sizes();
        assert_eq!(sizes.input, 1);
    }

    #[test]
    fn max_report_sizes_rounds_bits_up_and_sums_fields() {
        // One report with two fields: 5 bits x 3 (= 15 bits) plus 8 bits x 1 = 23 bits -> 3 bytes.
        // Also declares an output field of 8 bits x 2 = 2 bytes and a feature field of 16 bits.
        #[rustfmt::skip]
        let bytes = [
            0x05, 0x01,       // Usage Page (Generic Desktop)
            0xA1, 0x01,       // Collection (Application)
            0x85, 0x07,       //   Report ID (7)
            0x75, 0x05,       //   Report Size (5)
            0x95, 0x03,       //   Report Count (3)
            0x81, 0x02,       //   Input  (15 bits)
            0x75, 0x08,       //   Report Size (8)
            0x95, 0x01,       //   Report Count (1)
            0x81, 0x02,       //   Input  (+8 bits => 23 bits total => 3 bytes)
            0x95, 0x02,       //   Report Count (2)
            0x91, 0x02,       //   Output (8 bits x 2 => 2 bytes)
            0x75, 0x10,       //   Report Size (16)
            0x95, 0x01,       //   Report Count (1)
            0xB1, 0x02,       //   Feature (16 bits => 2 bytes)
            0xC0,             // End Collection
        ];
        let sizes = HidReportDescriptor::new(&bytes).unwrap().max_report_sizes();
        assert_eq!(
            sizes,
            MaxReportSizes {
                input: 3,
                output: 2,
                feature: 2
            }
        );
    }

    #[test]
    fn max_report_sizes_takes_max_across_report_ids() {
        // Two contiguously-described report IDs: ID 1 accumulates 8 + 8 = 16 bits (2 bytes), ID 2 is
        // 8 bits (1 byte). The larger report must win, proving the running maximum spans report IDs.
        #[rustfmt::skip]
        let bytes = [
            0xA1, 0x01,       // Collection (Application)
            0x75, 0x08,       //   Report Size (8)
            0x95, 0x01,       //   Report Count (1)
            0x85, 0x01,       //   Report ID (1)
            0x81, 0x02,       //   Input (8 bits into report 1)
            0x81, 0x02,       //   Input (+8 bits => 16 bits total in report 1)
            0x85, 0x02,       //   Report ID (2)
            0x81, 0x02,       //   Input (8 bits into report 2)
            0xC0,             // End Collection
        ];
        let sizes = HidReportDescriptor::new(&bytes).unwrap().max_report_sizes();
        assert_eq!(sizes.input, 2);
    }

    #[test]
    fn max_report_sizes_honors_push_pop() {
        // Push saves Report Size 8; a nested Report Size 32 is used for one field; Pop restores 8 so
        // the final field is 8 bits again. The report totals 8 + 32 + 8 = 48 bits = 6 bytes.
        #[rustfmt::skip]
        let bytes = [
            0xA1, 0x01,       // Collection (Application)
            0x85, 0x01,       //   Report ID (1)
            0x95, 0x01,       //   Report Count (1)
            0x75, 0x08,       //   Report Size (8)
            0x81, 0x02,       //   Input (8 bits)
            0xA4,             //   Push
            0x75, 0x20,       //     Report Size (32)
            0x81, 0x02,       //     Input (32 bits)
            0xB4,             //   Pop (restores Report Size 8)
            0x81, 0x02,       //   Input (8 bits)
            0xC0,             // End Collection
        ];
        let sizes = HidReportDescriptor::new(&bytes).unwrap().max_report_sizes();
        assert_eq!(sizes.input, 6);
    }

    #[test]
    fn max_report_sizes_unbalanced_pop_errors() {
        // A Pop with no matching Push is malformed and is rejected at construction.
        let bytes = [0xB4u8];
        let err = HidReportDescriptor::new(&bytes).unwrap_err();
        assert_eq!(err, HidDescriptorError::UnbalancedPop);
    }

    // ---- impl_hid_aggregate_device! ----

    /// A tiny mock [`HidDevice`] used to verify aggregate routing.
    ///
    /// `get_report` echoes back `[tag, received_report_id]` so tests can confirm both which child was
    /// reached (`tag`) and what native report ID it was addressed with. `set_report` returns an error
    /// unless it was addressed with this device's native report ID, so a routing/relabel bug surfaces
    /// as an error. Input reports are emitted with the device's native report ID and a `tag` payload.
    struct MockDev {
        descriptor: HidReportDescriptor<'static>,
        tag: u8,
        native_id: u8,
        has_input: bool,
        resets: u32,
        power: Option<HidDevicePowerState>,
    }

    impl MockDev {
        fn new(tag: u8, native_id: u8) -> Self {
            static DESC_ID1: [u8; 17] = explicit_descriptor(1);
            Self {
                descriptor: HidReportDescriptor::new(&DESC_ID1).unwrap(),
                tag,
                native_id,
                has_input: false,
                resets: 0,
                power: None,
            }
        }
    }

    impl HidDevice for MockDev {
        type InputReportMaxSize = typenum::U4;
        type OutputReportMaxSize = typenum::U4;
        type FeatureReportMaxSize = typenum::U0;
        const MAX_REPORT_COUNT: u8 = 2;
        // Comfortably bounds every descriptor the tests feed a `MockDev` (the largest is 17 bytes).
        const MAX_DESCRIPTOR_LEN: usize = 32;

        fn report_descriptor(&self) -> &HidReportDescriptor<'_> {
            &self.descriptor
        }

        async fn process_get_report<R>(
            &mut self,
            _report_type: GetHidReportType,
            report_id: ReportId,
            process_report: impl AsyncFnOnce(GetHidReport<'_>) -> R,
        ) -> Result<R, HidError> {
            let data = [self.tag, report_id.0];
            Ok(process_report(GetHidReport::Input(HidReport::new(report_id, &data))).await)
        }

        async fn set_report(&mut self, report: &SetHidReport<'_>) -> Result<(), HidError> {
            if report.id().0 == self.native_id {
                Ok(())
            } else {
                Err(HidError::TriggerReset)
            }
        }

        fn wait_for_input_report(&mut self) -> impl core::future::Future<Output = ()> {
            let ready = self.has_input;
            async move {
                if !ready {
                    core::future::pending::<()>().await
                }
            }
        }

        fn has_pending_input_report(&mut self) -> bool {
            self.has_input
        }

        async fn process_next_input_report<R>(
            &mut self,
            process_report: impl AsyncFnOnce(HidReport<'_>) -> R,
        ) -> Result<R, HidError> {
            self.has_input = false;
            let data = [self.tag, 0xEE];
            Ok(process_report(HidReport::new(ReportId(self.native_id), &data)).await)
        }

        async fn set_power_state(&mut self, state: HidDevicePowerState) -> Result<(), HidError> {
            self.power = Some(state);
            Ok(())
        }

        async fn reset(&mut self) {
            self.resets += 1;
        }
    }

    impl_hid_aggregate_device!(TestAggregate: MockDev, MockDev);

    /// Test-only accessor for a child device, reaching through the generated `ChildDevice` enum.
    /// Both variants wrap a `MockDev`, so a single or-pattern binds the inner device.
    fn child_mut<'a>(agg: &'a mut TestAggregate<'_>, i: usize) -> &'a mut MockDev {
        match &mut agg.children[i] {
            TestAggregateChildDevice::Variant0(d) | TestAggregateChildDevice::Variant1(d) => d,
        }
    }

    #[test]
    fn aggregate_combines_descriptor_and_report_sizes() {
        use typenum::Unsigned;

        let mut resources = TestAggregateResources::new();
        let agg = TestAggregate::new(&mut resources, MockDev::new(0, 1), MockDev::new(1, 1)).unwrap();

        // The first child keeps report ID 1; the conflicting second child is renumbered to 2.
        assert_eq!(&report_ids(agg.report_descriptor().as_bytes())[..], &[1, 2]);
        assert!(!agg.report_descriptor().report_ids_implicit());

        // Report sizes are the element-wise max, and the report count is the sum.
        assert_eq!(<TestAggregate<'static> as HidDevice>::InputReportMaxSize::to_usize(), 4);
        assert_eq!(
            <TestAggregate<'static> as HidDevice>::OutputReportMaxSize::to_usize(),
            4
        );
        assert_eq!(
            <TestAggregate<'static> as HidDevice>::FeatureReportMaxSize::to_usize(),
            0
        );
        assert_eq!(<TestAggregate<'static> as HidDevice>::MAX_REPORT_COUNT, 4);
    }

    #[test]
    fn aggregate_routes_get_report_and_relabels() {
        let mut resources = TestAggregateResources::new();
        let mut agg = TestAggregate::new(&mut resources, MockDev::new(0, 1), MockDev::new(1, 1)).unwrap();

        embassy_futures::block_on(async {
            // Host report ID 1 -> child 0 (tag 0), addressed with its native ID 1.
            let mut captured = [0u8; 2];
            agg.process_get_report(GetHidReportType::Input, ReportId(1), async |r| {
                captured.copy_from_slice(r.data());
            })
            .await
            .unwrap();
            assert_eq!(captured, [0, 1]);

            // Host report ID 2 -> child 1 (tag 1), relabelled back to its native ID 1.
            agg.process_get_report(GetHidReportType::Input, ReportId(2), async |r| {
                captured.copy_from_slice(r.data());
            })
            .await
            .unwrap();
            assert_eq!(captured, [1, 1]);

            // An unknown host report ID is rejected.
            assert!(
                agg.process_get_report(GetHidReportType::Input, ReportId(9), async |_r| {})
                    .await
                    .is_err()
            );
        });
    }

    #[test]
    fn aggregate_set_report_relabels_to_native_id() {
        let mut resources = TestAggregateResources::new();
        let mut agg = TestAggregate::new(&mut resources, MockDev::new(0, 1), MockDev::new(1, 1)).unwrap();

        embassy_futures::block_on(async {
            // Each child accepts only its native ID (1). Both host IDs must be relabelled to 1, so
            // both set_reports succeed; a relabel bug would deliver the host ID and cause an error.
            let out1 = [0xAAu8];
            agg.set_report(&SetHidReport::Output(HidReport::new(ReportId(1), &out1)))
                .await
                .unwrap();
            agg.set_report(&SetHidReport::Output(HidReport::new(ReportId(2), &out1)))
                .await
                .unwrap();

            // An unknown host report ID is rejected.
            assert!(
                agg.set_report(&SetHidReport::Output(HidReport::new(ReportId(9), &out1)))
                    .await
                    .is_err()
            );
        });
    }

    #[test]
    fn aggregate_relabels_input_reports() {
        let mut resources = TestAggregateResources::new();
        let mut agg = TestAggregate::new(&mut resources, MockDev::new(0, 1), MockDev::new(1, 1)).unwrap();

        // Mark the second child (renumbered to host ID 2) as having a pending input report.
        child_mut(&mut agg, 1).has_input = true;

        embassy_futures::block_on(async {
            assert!(agg.has_pending_input_report());
            let mut id = 0u8;
            let mut tag = 0u8;
            agg.process_next_input_report(async |r| {
                id = r.id().0;
                tag = r.data()[0];
            })
            .await
            .unwrap();
            // Child 1's native report ID (1) is relabelled to the host-facing ID (2), and it is indeed
            // child 1 that produced the report (tag 1).
            assert_eq!(id, 2);
            assert_eq!(tag, 1);
            assert!(!agg.has_pending_input_report());
        });
    }

    #[test]
    fn aggregate_fans_out_power_and_reset() {
        let mut resources = TestAggregateResources::new();
        let mut agg = TestAggregate::new(&mut resources, MockDev::new(0, 1), MockDev::new(1, 1)).unwrap();

        embassy_futures::block_on(async {
            agg.set_power_state(HidDevicePowerState::Sleep).await.unwrap();
            agg.reset().await;
        });

        // Both children observed the fanned-out commands.
        assert_eq!(child_mut(&mut agg, 0).power, Some(HidDevicePowerState::Sleep));
        assert_eq!(child_mut(&mut agg, 1).power, Some(HidDevicePowerState::Sleep));
        assert_eq!(child_mut(&mut agg, 0).resets, 1);
        assert_eq!(child_mut(&mut agg, 1).resets, 1);
    }
}
