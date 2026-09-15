use embedded_services::relay::hid;
use typenum::marker_traits::Unsigned;

/// HID descriptor as specified in section 5.1 of the HID-I2C spec. Not to be confused with a HID report descriptor, which
/// expresses the report types that the HID device can handle.  Field descriptions are taken directly from the HID-I2C spec.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, zerocopy::FromBytes, zerocopy::IntoBytes, zerocopy::Immutable)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct DeviceDescriptor {
    /// The length, in unsigned bytes, of the complete Hid Descriptor
    w_hid_desc_length: u16,

    /// The version number, in binary coded decimal (BCD) format. DEVICE should default to 0x0100
    bcd_version: u16,

    /// The length, in unsigned bytes, of the Report Descriptor.
    w_report_desc_length: u16,

    /// The register index containing the Report Descriptor on the DEVICE.
    w_report_desc_register: u16,

    /// This field identifies, in unsigned bytes, the register number to read the input report from the DEVICE.
    w_input_register: u16,

    /// This field identifies in unsigned bytes the length of the largest Input Report to be read from the Input Register (Complex HID Devices will need various sized reports).
    w_max_input_length: u16,

    /// This field identifies, in unsigned bytes, the register number to send the output report to the DEVICE.
    w_output_register: u16,

    /// This field identifies in unsigned bytes the length of the largest output Report to be sent to the Output Register (Complex HID Devices will need various sized reports).
    w_max_output_length: u16,

    /// This field identifies, in unsigned bytes, the register number to send command requests to the DEVICE
    w_command_register: u16,

    /// This field identifies in unsigned bytes the register number to exchange data with the Command Request
    w_data_register: u16,

    /// This field identifies the DEVICE manufacturers Vendor ID. Must be non-zero.
    w_vendor_id: u16,

    /// This field identifies the DEVICE’s unique model / Product ID.
    w_product_id: u16,

    /// This field identifies the DEVICE’s firmware revision number.
    w_version_id: u16,

    /// This field is reserved and should be set to 0.
    reserved: [u8; 4],
}

/// Hardware identifiers for the HID-I2C device
pub struct HardwareVersionInfo {
    pub vendor_id: VendorId,
    pub product_id: ProductId,
    pub version_id: VersionId,
}

/// Vendor ID, as assigned by the USB Implementers Forum (USB-IF).  Must be non-zero.
pub struct VendorId(u16);
impl VendorId {
    /// Creates a new VendorId.  Returns None if the vendor_id is invalid (i.e. zero).
    pub const fn new(vendor_id: u16) -> Option<Self> {
        if vendor_id == 0 { None } else { Some(Self(vendor_id)) }
    }

    /// The numeric value of the Vendor ID.
    pub const fn value(&self) -> u16 {
        self.0
    }
}

/// Product ID, as assigned by the device manufacturer.
pub struct ProductId(pub u16);

/// Version ID, as assigned by the device manufacturer. Recommended to be in BCD format, e.g. 0x0100 for version 1.00.
pub struct VersionId(pub u16);

/// Errors that can occur while constructing a `DeviceDescriptor` for a HID device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum DeviceDescriptorError {
    /// The HID device returned an input report descriptor whose largest report (`actual` bytes)
    /// is larger than the device's `InputReportMaxSize` (`max` bytes).
    InputReportTooLarge { actual: usize, max: usize },

    /// The HID device returned an output report descriptor whose largest report (`actual` bytes)
    /// is larger than the device's `OutputReportMaxSize` (`max` bytes).
    OutputReportTooLarge { actual: usize, max: usize },

    /// The HID device returned a feature report descriptor whose largest report (`actual` bytes)
    /// is larger than the device's `FeatureReportMaxSize` (`max` bytes).
    FeatureReportTooLarge { actual: usize, max: usize },

    /// The device declares a maximum report size that cannot be expressed in the descriptor's
    /// 16-bit `wMaxInputLength`/`wMaxOutputLength` fields once the length and report ID framing
    /// is added. Section 5.1 caps a report at `2^16 - 4` bytes.
    ReportTooLargeToFrame { max: usize },

    /// The report descriptor itself is longer than the 16-bit `wReportDescLength` field can
    /// express.
    ReportDescriptorTooLarge { actual: usize },
}

impl DeviceDescriptor {
    pub fn new<HidDevice: hid::HidDevice>(
        hid_device: &HidDevice,
        hwinfo: HardwareVersionInfo,
    ) -> Result<Self, DeviceDescriptorError> {
        const HID_I2C_PROTOCOL_VERSION: u16 = 0x0100;

        let descriptor = hid_device.report_descriptor();

        let actual_max_sizes = descriptor.max_report_sizes();
        let input_max = HidDevice::InputReportMaxSize::USIZE;
        if actual_max_sizes.input > input_max {
            return Err(DeviceDescriptorError::InputReportTooLarge {
                actual: actual_max_sizes.input,
                max: input_max,
            });
        }

        let output_max = HidDevice::OutputReportMaxSize::USIZE;
        if actual_max_sizes.output > output_max {
            return Err(DeviceDescriptorError::OutputReportTooLarge {
                actual: actual_max_sizes.output,
                max: output_max,
            });
        }

        if actual_max_sizes.feature > HidDevice::FeatureReportMaxSize::USIZE {
            return Err(DeviceDescriptorError::FeatureReportTooLarge {
                actual: actual_max_sizes.feature,
                max: HidDevice::FeatureReportMaxSize::USIZE,
            });
        }

        let report_length_header_size = crate::wire::ReportFraming::of(descriptor).header_bytes();

        // Section 5.1 caps a report at 2^16 - 4 bytes, and wMax*Length must also cover the length
        // field and the report ID. Anything that doesn't fit is a mis-declared device, not a
        // runtime condition, so it's rejected here rather than silently truncated by an `as` cast.
        let framed_len = |max: usize| -> Result<u16, DeviceDescriptorError> {
            u16::try_from(max)
                .ok()
                .and_then(|max| max.checked_add(report_length_header_size))
                .ok_or(DeviceDescriptorError::ReportTooLargeToFrame { max })
        };

        let report_desc_length = u16::try_from(descriptor.as_bytes().len()).map_err(|_| {
            DeviceDescriptorError::ReportDescriptorTooLarge {
                actual: descriptor.as_bytes().len(),
            }
        })?;

        Ok(Self {
            w_hid_desc_length: core::mem::size_of::<DeviceDescriptor>() as u16,
            bcd_version: HID_I2C_PROTOCOL_VERSION,
            w_report_desc_length: report_desc_length,
            w_report_desc_register: crate::HidI2cRegister::ReportDescriptor as u16,
            w_input_register: crate::HidI2cRegister::Input.into(),
            w_max_input_length: framed_len(input_max)?,
            w_output_register: crate::HidI2cRegister::Output.into(),
            w_max_output_length: framed_len(output_max)?,
            w_command_register: crate::HidI2cRegister::Command.into(),
            w_data_register: crate::HidI2cRegister::Data.into(),
            w_vendor_id: hwinfo.vendor_id.value(),
            w_product_id: hwinfo.product_id.0,
            w_version_id: hwinfo.version_id.0,
            reserved: [0; 4],
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::test_support::{descriptor_device, hardware_version_info};

    const IMPLICIT_DESCRIPTOR: &[u8] = &[
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

    const EXPLICIT_DESCRIPTOR: &[u8] = &[
        0x05, 0x01, // Usage Page (Generic Desktop)
        0x09, 0x02, // Usage (Mouse)
        0xa1, 0x01, // Collection (Application)
        0x85, 0x01, // Report ID (1)
        0x75, 0x08, // Report Size (8)
        0x95, 0x01, // Report Count (1)
        0x81, 0x02, // Input
        0x91, 0x02, // Output
        0xb1, 0x02, // Feature
        0xc0, // End Collection
    ];

    const TWO_BYTE_INPUT_DESCRIPTOR: &[u8] = &[
        0x05, 0x01, // Usage Page (Generic Desktop)
        0x09, 0x02, // Usage (Mouse)
        0xa1, 0x01, // Collection (Application)
        0x75, 0x08, // Report Size (8)
        0x95, 0x02, // Report Count (2)
        0x81, 0x02, // Input
        0xc0, // End Collection
    ];

    const TWO_BYTE_OUTPUT_DESCRIPTOR: &[u8] = &[
        0x05, 0x01, // Usage Page (Generic Desktop)
        0x09, 0x02, // Usage (Mouse)
        0xa1, 0x01, // Collection (Application)
        0x75, 0x08, // Report Size (8)
        0x95, 0x02, // Report Count (2)
        0x91, 0x02, // Output
        0xc0, // End Collection
    ];

    const TWO_BYTE_FEATURE_DESCRIPTOR: &[u8] = &[
        0x05, 0x01, // Usage Page (Generic Desktop)
        0x09, 0x02, // Usage (Mouse)
        0xa1, 0x01, // Collection (Application)
        0x75, 0x08, // Report Size (8)
        0x95, 0x02, // Report Count (2)
        0xb1, 0x02, // Feature
        0xc0, // End Collection
    ];

    #[test]
    fn descriptor_uses_implicit_report_framing() {
        let descriptor =
            DeviceDescriptor::new(&descriptor_device(IMPLICIT_DESCRIPTOR), hardware_version_info()).unwrap();

        assert_eq!(
            descriptor.w_hid_desc_length,
            core::mem::size_of::<DeviceDescriptor>() as u16
        );
        assert_eq!(descriptor.bcd_version, 0x0100);
        assert_eq!(descriptor.w_report_desc_length, IMPLICIT_DESCRIPTOR.len() as u16);
        assert_eq!(descriptor.w_max_input_length, 3);
        assert_eq!(descriptor.w_max_output_length, 3);
        assert_eq!(descriptor.w_vendor_id, 0x1234);
        assert_eq!(descriptor.w_product_id, 0x5678);
        assert_eq!(descriptor.w_version_id, 0x0100);
    }

    #[test]
    fn descriptor_accounts_for_explicit_report_id() {
        let descriptor =
            DeviceDescriptor::new(&descriptor_device(EXPLICIT_DESCRIPTOR), hardware_version_info()).unwrap();

        assert_eq!(descriptor.w_max_input_length, 4);
        assert_eq!(descriptor.w_max_output_length, 4);
    }

    #[test]
    fn descriptor_rejects_oversized_input_report() {
        let result = DeviceDescriptor::new(&descriptor_device(TWO_BYTE_INPUT_DESCRIPTOR), hardware_version_info());

        assert_eq!(
            result,
            Err(DeviceDescriptorError::InputReportTooLarge { actual: 2, max: 1 })
        );
    }

    #[test]
    fn descriptor_rejects_oversized_output_report() {
        let result = DeviceDescriptor::new(&descriptor_device(TWO_BYTE_OUTPUT_DESCRIPTOR), hardware_version_info());

        assert_eq!(
            result,
            Err(DeviceDescriptorError::OutputReportTooLarge { actual: 2, max: 1 })
        );
    }

    #[test]
    fn descriptor_rejects_oversized_feature_report() {
        let result = DeviceDescriptor::new(&descriptor_device(TWO_BYTE_FEATURE_DESCRIPTOR), hardware_version_info());

        assert_eq!(
            result,
            Err(DeviceDescriptorError::FeatureReportTooLarge { actual: 2, max: 1 })
        );
    }

    #[test]
    fn vendor_id_rejects_zero() {
        assert!(VendorId::new(0).is_none());
        assert_eq!(VendorId::new(1).unwrap().value(), 1);
    }
}
