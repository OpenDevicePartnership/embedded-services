//! Module to contain OEM-specific definitions

pub mod vendor;

/// Vendor ID
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[repr(transparent)]
pub struct VendorId(pub u16);

/// Header for generic OEM messages
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct MessageHeader {
    /// Target vendor for this message
    pub vendor: VendorId,
    /// Vendor-spcific value
    pub function: u16,
}

impl MessageHeader {
    /// Create new OEM message header
    pub fn new(vendor: VendorId, function: u16) -> Self {
        Self { vendor, function }
    }
}

/// Data for generic OEM messages
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum MessageData {
    /// A single bool value
    Bool(bool),

    /// A single u8 value
    U8(u8),
    /// A single u16 value
    U16(u16),
    /// A single u32 value
    U32(u32),
    /// A single u64 value
    U64(u64),
    /// A single i8 value
    I8(i8),
    /// A single i16 value
    I16(i16),
    /// A single i32 value
    I32(i32),
    /// A single i64 value
    I64(i64),

    /// A single usize value
    Usize(usize),
    /// A single isize value
    Isize(isize),

    /// A single f32 value
    F32(f32),

    /// A single string slice
    Str(&'static str),

    /// Arbitrary data
    Bytes(&'static [u8]),
}

/// Generic OEM message
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Message {
    /// Message header
    pub header: MessageHeader,
    /// Message data
    pub data: MessageData,
}

impl Message {
    /// Create a new OEM message
    pub fn new(vendor: VendorId, function: u16, data: MessageData) -> Self {
        let header = MessageHeader::new(vendor, function);
        Self { header, data }
    }
}
