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

/// The number of bytes in a HID report header, which consists of a 2-byte length field.
pub const HID_REPORT_HEADER_SIZE_BYTES: u16 = 2;

/// The number of bytes in a HID report ID field, which consists of a 1-byte report ID.
/// This field is only used if more than one report of any type is exposed by the HID device (i.e. you can
/// have a single input report, a single output report, and a single feature report and not need this, but
/// as soon as you add a second of any one of those you need this).
pub const HID_REPORT_ID_SIZE_BYTES: u16 = 1;

impl DeviceDescriptor {
    pub fn new<HidDevice: hid::HidDevice>(hid_device: &HidDevice, hwinfo: HardwareVersionInfo) -> Self {
        const HID_I2C_PROTOCOL_VERSION: u16 = 0x0100;
        Self {
            w_hid_desc_length: core::mem::size_of::<DeviceDescriptor>() as u16,
            bcd_version: HID_I2C_PROTOCOL_VERSION,
            w_report_desc_length: hid_device.report_descriptor().as_bytes().len() as u16,
            w_report_desc_register: crate::HidI2cRegister::ReportDescriptor as u16,
            w_input_register: crate::HidI2cRegister::Input.into(),
            w_max_input_length: HidDevice::InputReportMaxSize::USIZE as u16
                + HID_REPORT_HEADER_SIZE_BYTES
                + if hid_device.report_descriptor().report_ids_implicit() {
                    0
                } else {
                    HID_REPORT_ID_SIZE_BYTES
                },
            w_output_register: crate::HidI2cRegister::Output.into(),
            w_max_output_length: HidDevice::OutputReportMaxSize::USIZE as u16
                + HID_REPORT_HEADER_SIZE_BYTES
                + if hid_device.report_descriptor().report_ids_implicit() {
                    0
                } else {
                    HID_REPORT_ID_SIZE_BYTES
                },
            w_command_register: crate::HidI2cRegister::Command.into(),
            w_data_register: crate::HidI2cRegister::Data.into(),
            w_vendor_id: hwinfo.vendor_id.value(),
            w_product_id: hwinfo.product_id.0,
            w_version_id: hwinfo.version_id.0,
            reserved: [0; 4],
        }
    }
}
