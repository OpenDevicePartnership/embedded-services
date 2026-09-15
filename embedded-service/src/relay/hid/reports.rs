//! Bit-level packing and unpacking helper for HID reports.
//!
//! HID lays report fields out as a little-endian bit stream: the first field starts at bit 0 of
//! byte 0, each subsequent field starts where the previous one ended, and fields straddle
//! byte boundaries.  Signed fields are also supported, and accessing them requires doing sign extension.
//! This means that for reports that don't happen to be byte-aligned (e.g. boot protocol mouse/keyboard reports),
//! #[repr(C)] structs aren't sufficient to handle reports and you need to do some bit-level packing and unpacking manually.
//!
//! This module provides the following utilities for packing/unpacking reports that can't just be #[repr(C)] structs:
//!
//! * [`get_bits`] / [`set_bits`] for ad-hoc access to a bit range in a buffer.
//! * [`hid_report!`](crate::hid_report) to declare a report as a normal Rust struct with per-field
//!   bit widths, generating `pack`/`pack_into`/`unpack`.
//!
//! Report IDs are *not* handled here; strip or prepend the ID byte in the caller.
//!
//! ```ignore
//! hid_report! {
//!     /// Report ID 3 (Time and Date), output direction.
//!     pub struct SetTimeReport {
//!         /// 1900 - 9999
//!         pub year: u16 => 14,
//!         /// 1 - 12
//!         pub month: u8 => 4,
//!         /// 1 - 31
//!         pub day: u8 => 5,
//!         /// 0 - 23
//!         pub hour: u8 => 5,
//!         /// 0 - 59
//!         pub minute: u8 => 6,
//!         /// 0 - 59
//!         pub second: u8 => 6,
//!         /// 0 - 999
//!         pub millisecond: u16 => 10,
//!         /// -720 - 840, minutes from UTC
//!         pub time_zone: i16 => 11,
//!         pub dst_observed: bool => 1,
//!         pub dst_active: bool => 1,
//!         pub reserved: u8 => 1,
//!     }
//! }
//!
//! let report = SetTimeReport::unpack(payload)?;
//! let bytes = report.pack()?;
//! ```

/// Failure modes for packing and unpacking report fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum ReportError {
    /// The requested bit range extends past the end of the buffer.
    OutOfBounds,

    /// The requested width is zero, wider than 32 bits, or wider than the field's storage type.
    InvalidWidth,

    /// The value does not fit in the requested number of bits.
    ValueOutOfRange,
}

/// Mask with the low `width` bits set.
const fn mask(width: u32) -> u32 {
    if width >= u32::BITS {
        u32::MAX
    } else {
        (1u32 << width) - 1
    }
}

/// Interprets the low `width` bits of `raw` as a two's-complement signed value.
const fn sign_extend(raw: u32, width: u32) -> i32 {
    if width >= u32::BITS {
        raw as i32
    } else {
        let shift = u32::BITS - width;
        ((raw << shift) as i32) >> shift
    }
}

/// Inclusive range representable by a two's-complement field of `width` bits. `width` must be 1..=32.
const fn signed_range(width: u32) -> (i32, i32) {
    if width >= u32::BITS {
        (i32::MIN, i32::MAX)
    } else {
        let magnitude = 1i32 << (width - 1);
        (-magnitude, magnitude - 1)
    }
}

fn check_bounds(data_len: usize, offset: usize, width: u32) -> Result<(), ReportError> {
    if width == 0 || width > u32::BITS {
        return Err(ReportError::InvalidWidth);
    }

    let end = offset.checked_add(width as usize).ok_or(ReportError::OutOfBounds)?;
    if end.div_ceil(8) > data_len {
        return Err(ReportError::OutOfBounds);
    }

    Ok(())
}

/// Reads `width` bits starting at bit `offset` (LSB of byte 0 is bit 0), returned right-aligned.
pub fn get_bits(data: &[u8], offset: usize, width: u32) -> Result<u32, ReportError> {
    check_bounds(data.len(), offset, width)?;

    let mut value = 0u32;
    for bit in 0..width as usize {
        let pos = offset + bit;
        let byte = *data.get(pos / 8).ok_or(ReportError::OutOfBounds)?;
        if byte & (1u8 << (pos % 8)) != 0 {
            value |= 1u32 << bit;
        }
    }

    Ok(value)
}

/// Writes the low `width` bits of `value` starting at bit `offset`. Bits outside the range are left alone.
pub fn set_bits(data: &mut [u8], offset: usize, width: u32, value: u32) -> Result<(), ReportError> {
    check_bounds(data.len(), offset, width)?;

    if value & !mask(width) != 0 {
        return Err(ReportError::ValueOutOfRange);
    }

    for bit in 0..width as usize {
        let pos = offset + bit;
        let byte = data.get_mut(pos / 8).ok_or(ReportError::OutOfBounds)?;
        let bit_mask = 1u8 << (pos % 8);
        if value & (1u32 << bit) != 0 {
            *byte |= bit_mask;
        } else {
            *byte &= !bit_mask;
        }
    }

    Ok(())
}

/// A type that can be stored in a HID report bitfield.
pub trait ReportField: Copy + Sized {
    /// Width of the type this field is stored in; a field may not be declared wider than this.
    const STORAGE_BITS: u32;

    /// Converts the right-aligned raw bits of a `width`-bit field into this type.
    fn from_bits(raw: u32, width: u32) -> Result<Self, ReportError>;

    /// Converts this value into the right-aligned raw bits of a `width`-bit field.
    fn to_bits(self, width: u32) -> Result<u32, ReportError>;
}

fn check_width<T: ReportField>(width: u32) -> Result<(), ReportError> {
    if width == 0 || width > u32::BITS || width > T::STORAGE_BITS {
        Err(ReportError::InvalidWidth)
    } else {
        Ok(())
    }
}

macro_rules! impl_unsigned_field {
    ($($ty:ty),* $(,)?) => {
        $(
            impl ReportField for $ty {
                const STORAGE_BITS: u32 = <$ty>::BITS;

                fn from_bits(raw: u32, width: u32) -> Result<Self, ReportError> {
                    check_width::<Self>(width)?;
                    Ok((raw & mask(width)) as $ty)
                }

                fn to_bits(self, width: u32) -> Result<u32, ReportError> {
                    check_width::<Self>(width)?;
                    let value = u32::from(self);
                    if value & !mask(width) != 0 {
                        return Err(ReportError::ValueOutOfRange);
                    }
                    Ok(value)
                }
            }
        )*
    };
}

macro_rules! impl_signed_field {
    ($($ty:ty),* $(,)?) => {
        $(
            impl ReportField for $ty {
                const STORAGE_BITS: u32 = <$ty>::BITS;

                fn from_bits(raw: u32, width: u32) -> Result<Self, ReportError> {
                    check_width::<Self>(width)?;
                    Ok(sign_extend(raw, width) as $ty)
                }

                fn to_bits(self, width: u32) -> Result<u32, ReportError> {
                    check_width::<Self>(width)?;
                    let value = i32::from(self);
                    let (min, max) = signed_range(width);
                    if value < min || value > max {
                        return Err(ReportError::ValueOutOfRange);
                    }
                    Ok((value as u32) & mask(width))
                }
            }
        )*
    };
}

impl_unsigned_field!(u8, u16, u32);
impl_signed_field!(i8, i16, i32);

impl ReportField for bool {
    const STORAGE_BITS: u32 = 1;

    fn from_bits(raw: u32, width: u32) -> Result<Self, ReportError> {
        check_width::<Self>(width)?;
        Ok(raw & 1 != 0)
    }

    fn to_bits(self, width: u32) -> Result<u32, ReportError> {
        check_width::<Self>(width)?;
        Ok(u32::from(self))
    }
}

/// Declares a HID report struct and generates its bit-level (de)serialization.
///
/// Fields are laid out LSB-first in declaration order, matching the order of `Input`/`Output`/
/// `Feature` items in the report descriptor. Every field of the descriptor must be declared,
/// including constant/padding items, so that the offsets line up.
///
/// Field syntax is `name: storage_type => bit_width`. Supported storage types are `u8`, `u16`,
/// `u32`, `i8`, `i16`, `i32` (two's complement) and `bool` (exactly one bit).
///
/// The declared widths must sum to a whole number of bytes, which is checked at compile time;
/// declare the report descriptor's constant/padding items to make up any shortfall.
///
/// The Windows HID APIs don't support field sizes larger than 32 bits, so if you need larger values,
/// you have to split them into multiple fields.
#[macro_export]
macro_rules! hid_report {
    (
        $(#[$meta:meta])*
        $vis:vis struct $name:ident {
            $(
                $(#[$field_meta:meta])*
                $field_vis:vis $field:ident : $ty:ty => $bits:expr
            ),* $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, Default, PartialEq)]
        #[cfg_attr(feature = "defmt", derive(defmt::Format))]
        #[allow(dead_code)]
        $vis struct $name {
            $(
                $(#[$field_meta])*
                $field_vis $field: $ty,
            )*
        }

        #[allow(dead_code)]
        impl $name {
            /// Total width of the report in bits, including padding fields.
            pub const BIT_LEN: usize = 0 $( + ($bits as usize) )*;

            /// Size of the packed report in bytes, excluding any report ID prefix.
            pub const BYTE_LEN: usize = $name::BIT_LEN.div_ceil(8);

            /// Bit offset of each field, in declaration order.
            #[allow(dead_code, clippy::indexing_slicing)]
            pub const FIELD_OFFSETS: [usize; 0 $( + { let _ = $bits; 1 } )*] = {
                let mut offsets = [0usize; 0 $( + { let _ = $bits; 1 } )*];
                let mut offset = 0usize;
                let mut index = 0usize;
                $(
                    offsets[index] = offset;
                    offset += ($bits as usize);
                    index += 1;
                )*
                let _ = (offset, index);
                offsets
            };

            /// Packs the report into a new buffer.
            pub fn pack(&self) -> Result<[u8; $name::BYTE_LEN], $crate::relay::hid::reports::ReportError> {
                let mut buffer = [0u8; $name::BYTE_LEN];
                self.pack_into(&mut buffer)?;
                Ok(buffer)
            }

            /// Packs the report into the start of `buffer`, returning the number of bytes written.
            ///
            /// Bytes past the end of the report are left untouched.
            pub fn pack_into(&self, buffer: &mut [u8]) -> Result<usize, $crate::relay::hid::reports::ReportError> {
                #[allow(unused_mut, unused_variables)]
                let mut offset: usize = 0;
                $(
                    $crate::relay::hid::reports::set_bits(
                        buffer,
                        offset,
                        $bits,
                        $crate::relay::hid::reports::ReportField::to_bits(self.$field, $bits)?,
                    )?;
                    offset += ($bits as usize);
                )*
                let _ = offset;
                Ok($name::BYTE_LEN)
            }

            /// Unpacks the report from the start of `buffer`; trailing bytes are ignored.
            pub fn unpack(buffer: &[u8]) -> Result<Self, $crate::relay::hid::reports::ReportError> {
                #[allow(unused_mut, unused_variables)]
                let mut offset: usize = 0;
                $(
                    let $field = <$ty as $crate::relay::hid::reports::ReportField>::from_bits(
                        $crate::relay::hid::reports::get_bits(buffer, offset, $bits)?,
                        $bits,
                    )?;
                    offset += ($bits as usize);
                )*
                let _ = offset;
                Ok(Self { $( $field, )* })
            }
        }

        const _: () = assert!(
            $name::BIT_LEN % 8 == 0,
            concat!(
                stringify!($name),
                ": HID report field widths must sum to a whole number of bytes - declare the \
                 descriptor's constant/padding items to make up the difference",
            ),
        );
    };
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    crate::hid_report! {
        pub struct TimeReport {
            pub year: u16 => 14,
            pub month: u8 => 4,
            pub day: u8 => 5,
            pub hour: u8 => 5,
            pub minute: u8 => 6,
            pub second: u8 => 6,
            pub millisecond: u16 => 10,
            pub time_zone: i16 => 11,
            pub dst_observed: bool => 1,
            pub dst_active: bool => 1,
            pub reserved: u8 => 1,
        }
    }

    #[test]
    fn report_geometry() {
        assert_eq!(TimeReport::BIT_LEN, 64);
        assert_eq!(TimeReport::BYTE_LEN, 8);
        assert_eq!(TimeReport::FIELD_OFFSETS, [0, 14, 18, 23, 28, 34, 40, 50, 61, 62, 63]);
    }

    #[test]
    fn round_trip() {
        let report = TimeReport {
            year: 2026,
            month: 9,
            day: 1,
            hour: 13,
            minute: 45,
            second: 59,
            millisecond: 999,
            time_zone: -480,
            dst_observed: true,
            dst_active: false,
            reserved: 0,
        };

        let packed = report.pack().unwrap();
        assert_eq!(TimeReport::unpack(&packed).unwrap(), report);
    }

    #[test]
    fn unpack_ignores_trailing_bytes() {
        let report = TimeReport {
            year: 1900,
            time_zone: 840,
            ..Default::default()
        };

        let mut buffer = [0xAAu8; 12];
        assert_eq!(report.pack_into(&mut buffer).unwrap(), 8);
        assert_eq!(TimeReport::unpack(&buffer).unwrap(), report);
        assert_eq!(buffer[8..], [0xAA; 4]);
    }

    #[test]
    fn straddles_byte_boundaries() {
        // year (14 bits) then month (4 bits) starting mid-byte.
        let report = TimeReport {
            year: 0x3FFF,
            month: 0xF,
            ..Default::default()
        };

        let packed = report.pack().unwrap();
        assert_eq!(packed[0], 0xFF);
        assert_eq!(packed[1], 0xFF);
        assert_eq!(packed[2], 0x03);
    }

    #[test]
    fn rejects_oversized_values() {
        let report = TimeReport {
            month: 16,
            ..Default::default()
        };
        assert_eq!(report.pack(), Err(ReportError::ValueOutOfRange));

        let report = TimeReport {
            time_zone: 1024,
            ..Default::default()
        };
        assert_eq!(report.pack(), Err(ReportError::ValueOutOfRange));
    }

    #[test]
    fn rejects_short_buffers() {
        let mut buffer = [0u8; 7];
        assert_eq!(
            TimeReport::default().pack_into(&mut buffer),
            Err(ReportError::OutOfBounds)
        );
        assert_eq!(TimeReport::unpack(&buffer), Err(ReportError::OutOfBounds));
    }

    #[test]
    fn bit_accessors() {
        let data = [0b1010_0101u8, 0b0000_0011];
        assert_eq!(get_bits(&data, 0, 4).unwrap(), 0b0101);
        assert_eq!(get_bits(&data, 4, 4).unwrap(), 0b1010);
        assert_eq!(get_bits(&data, 6, 4).unwrap(), 0b1110);
        assert_eq!(get_bits(&data, 0, 16).unwrap(), 0x03A5);

        let mut data = [0u8; 2];
        set_bits(&mut data, 6, 4, 0b1110).unwrap();
        assert_eq!(data, [0b1000_0000, 0b0000_0011]);
        set_bits(&mut data, 6, 4, 0).unwrap();
        assert_eq!(data, [0, 0]);
    }

    #[test]
    fn bit_accessor_errors() {
        let mut data = [0u8; 2];
        assert_eq!(get_bits(&data, 0, 0), Err(ReportError::InvalidWidth));
        assert_eq!(get_bits(&data, 0, 33), Err(ReportError::InvalidWidth));
        assert_eq!(get_bits(&data, 9, 8), Err(ReportError::OutOfBounds));
        assert_eq!(set_bits(&mut data, 0, 4, 0x10), Err(ReportError::ValueOutOfRange));
    }

    #[test]
    fn signed_fields_sign_extend() {
        assert_eq!(<i16 as ReportField>::from_bits(0b111_1010_0000, 11).unwrap(), -96);
        assert_eq!(<i16 as ReportField>::to_bits(-96, 11).unwrap(), 0b111_1010_0000);
        assert_eq!(
            <i16 as ReportField>::to_bits(-1025, 11),
            Err(ReportError::ValueOutOfRange)
        );
    }

    #[test]
    fn width_must_fit_storage_type() {
        assert_eq!(<u8 as ReportField>::from_bits(0, 9), Err(ReportError::InvalidWidth));
        assert_eq!(<bool as ReportField>::to_bits(true, 2), Err(ReportError::InvalidWidth));
    }
}
