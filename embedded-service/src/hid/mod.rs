//! HID sevices
//! See spec at http://msdn.microsoft.com/en-us/library/windows/hardware/hh852380.aspx
use core::convert::Infallible;

use embassy_sync::once_lock::OnceLock;
use embassy_sync::signal::Signal;

use crate::buffer::SharedRef;
use crate::comms::{self, Endpoint, EndpointID, External, Internal, MailboxDelegate};
use crate::{GlobalRawMutex, IntrusiveList, Node, NodeContainer, error, intrusive_list};

mod command;
pub use command::*;

/// HID descriptor length
pub const DESCRIPTOR_LEN: usize = 30;

/// HID errors
#[derive(Clone, Copy, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Error {
    /// Invalid data
    InvalidData,
    /// Invalid size: expected and actual sizes
    InvalidSize(usize, usize),
    /// Invalid register address
    InvalidRegisterAddress,
    /// Invalid device
    InvalidDevice,
    /// Invalid command
    InvalidCommand,
    /// Command requires a report ID
    RequiresReportId,
    /// Command requires data
    RequiresData,
    /// Invalid report type for command
    InvalidReportType,
    /// Invalid report frequency
    InvalidReportFreq,
    /// Error from transport service
    Transport,
    /// Timeout
    Timeout,
    /// Errors from serialization/deserialization
    Serialize,
}

/// HID descriptor, see spec for descriptions
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[allow(missing_docs)]
pub struct Descriptor {
    pub w_hid_desc_length: u16,
    pub bcd_version: u16,
    pub w_report_desc_length: u16,
    pub w_report_desc_register: u16,
    pub w_input_register: u16,
    pub w_max_input_length: u16,
    pub w_output_register: u16,
    pub w_max_output_length: u16,
    pub w_command_register: u16,
    pub w_data_register: u16,
    pub w_vendor_id: u16,
    pub w_product_id: u16,
    pub w_version_id: u16,
}

impl Descriptor {
    /// Serializes a descriptor into the slice
    pub fn encode_into_slice(&self, buf: &mut [u8]) -> Result<usize, Error> {
        if buf.len() < DESCRIPTOR_LEN {
            return Err(Error::InvalidSize(DESCRIPTOR_LEN, buf.len()));
        }

        buf[0..2].copy_from_slice(&self.w_hid_desc_length.to_le_bytes());
        buf[2..4].copy_from_slice(&self.bcd_version.to_le_bytes());
        buf[4..6].copy_from_slice(&self.w_report_desc_length.to_le_bytes());
        buf[6..8].copy_from_slice(&self.w_report_desc_register.to_le_bytes());
        buf[8..10].copy_from_slice(&self.w_input_register.to_le_bytes());
        buf[10..12].copy_from_slice(&self.w_max_input_length.to_le_bytes());
        buf[12..14].copy_from_slice(&self.w_output_register.to_le_bytes());
        buf[14..16].copy_from_slice(&self.w_max_output_length.to_le_bytes());
        buf[16..18].copy_from_slice(&self.w_command_register.to_le_bytes());
        buf[18..20].copy_from_slice(&self.w_data_register.to_le_bytes());
        buf[20..22].copy_from_slice(&self.w_vendor_id.to_le_bytes());
        buf[22..24].copy_from_slice(&self.w_product_id.to_le_bytes());
        buf[24..26].copy_from_slice(&self.w_version_id.to_le_bytes());
        // Reserved
        buf[26..30].copy_from_slice(&[0u8; 4]);

        Ok(30)
    }

    /// Deserializes a descriptor from the slice
    pub fn decode_from_slice(buf: &[u8]) -> Result<Self, Error> {
        if buf.len() < DESCRIPTOR_LEN {
            return Err(Error::InvalidSize(DESCRIPTOR_LEN, buf.len()));
        }

        // Reserved bytes must be zero
        if buf[26..30] != [0u8; 4] {
            return Err(Error::InvalidData);
        }

        let descriptor = Descriptor {
            w_hid_desc_length: u16::from_le_bytes([buf[0], buf[1]]),
            bcd_version: u16::from_le_bytes([buf[2], buf[3]]),
            w_report_desc_length: u16::from_le_bytes([buf[4], buf[5]]),
            w_report_desc_register: u16::from_le_bytes([buf[6], buf[7]]),
            w_input_register: u16::from_le_bytes([buf[8], buf[9]]),
            w_max_input_length: u16::from_le_bytes([buf[10], buf[11]]),
            w_output_register: u16::from_le_bytes([buf[12], buf[13]]),
            w_max_output_length: u16::from_le_bytes([buf[14], buf[15]]),
            w_command_register: u16::from_le_bytes([buf[16], buf[17]]),
            w_data_register: u16::from_le_bytes([buf[18], buf[19]]),
            w_vendor_id: u16::from_le_bytes([buf[20], buf[21]]),
            w_product_id: u16::from_le_bytes([buf[22], buf[23]]),
            w_version_id: u16::from_le_bytes([buf[24], buf[25]]),
        };

        Ok(descriptor)
    }
}

/// HID register values
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RegisterFile {
    /// HID descriptor register
    pub hid_desc_reg: u16,
    /// HID report descriptor register
    pub report_desc_reg: u16,
    /// HID input report register
    pub input_reg: u16,
    /// HID output report register
    pub output_reg: u16,
    /// HID command register
    pub command_reg: u16,
    /// HID data register
    pub data_reg: u16,
}

/// HID devices commonly start with the descriptor register and increment from there in this order
impl Default for RegisterFile {
    fn default() -> Self {
        Self {
            hid_desc_reg: 0x0001,
            report_desc_reg: 0x0002,
            input_reg: 0x0003,
            output_reg: 0x0004,
            command_reg: 0x0005,
            data_reg: 0x0006,
        }
    }
}

/// HID device that responds to HID requests
pub struct Device {
    node: Node,
    tp: Endpoint,
    request: Signal<GlobalRawMutex, Request<'static>>,
    /// Device ID
    pub id: DeviceId,
    /// Registers
    pub regs: RegisterFile,
}

/// Trait to allow access to underlying Device
pub trait DeviceContainer {
    /// Get a reference to the underlying HID device
    fn get_hid_device(&self) -> &Device;
}

impl NodeContainer for Device {
    fn get_node(&self) -> &Node {
        &self.node
    }
}

impl Device {
    /// Instantiates a new device
    pub fn new(id: DeviceId, regs: RegisterFile) -> Self {
        Self {
            node: Node::uninit(),
            tp: Endpoint::uninit(EndpointID::Internal(Internal::Hid)),
            request: Signal::new(),
            id,
            regs,
        }
    }

    /// Wait for this device to receive a request
    pub async fn wait_request(&self) -> Request<'static> {
        self.request.wait().await
    }

    /// Send a response to the host from this device
    pub async fn send_response(&self, response: Option<Response<'static>>) -> Result<(), Infallible> {
        let message = Message {
            id: self.id,
            data: MessageData::Response(response),
        };
        self.tp.send(EndpointID::External(External::Host), &message).await
    }
}

impl DeviceContainer for Device {
    fn get_hid_device(&self) -> &Device {
        self
    }
}

impl MailboxDelegate for Device {
    fn receive(&self, message: &comms::Message) -> Result<(), comms::MailboxDelegateError> {
        let message = message
            .data
            .get::<Message>()
            .ok_or(comms::MailboxDelegateError::MessageNotFound)?;

        match message.data {
            MessageData::Request(ref request) => {
                self.request.signal(request.clone());
                Ok(())
            }
            _ if message.id != self.id => Err(comms::MailboxDelegateError::InvalidId),
            _ => Err(comms::MailboxDelegateError::InvalidData),
        }
    }
}

/// HID device ID
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct DeviceId(pub u8);

/// HID report ID
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct ReportId(pub u8);

/// Host to device messages
#[derive(Clone)]
pub enum Request<'a> {
    /// HID descriptor request
    Descriptor,
    /// Report descriptor request
    ReportDescriptor,
    /// Input report request
    InputReport,
    /// Output report request
    OutputReport(Option<ReportId>, SharedRef<'a, u8>),
    /// Command
    Command(Command<'a>),
}

/// Device to host messages
#[derive(Clone)]
pub enum Response<'a> {
    /// HID descriptor response
    Descriptor(SharedRef<'a, u8>),
    /// Report descriptor response
    ReportDescriptor(SharedRef<'a, u8>),
    /// Input report
    InputReport(SharedRef<'a, u8>),
    /// Feature report
    FeatureReport(SharedRef<'a, u8>),
    /// General command responses
    Command(CommandResponse),
}

/// HID message data
#[derive(Clone)]
pub enum MessageData<'a> {
    /// HID read/write request to register
    Request(Request<'a>),
    /// HID response, some commands may not produce a response
    Response(Option<Response<'a>>),
}

/// Top-level struct for HID communication
#[derive(Clone)]
pub struct Message<'a> {
    /// Target/originating device ID
    pub id: DeviceId,
    /// Message contents
    pub data: MessageData<'a>,
}

struct Context {
    devices: IntrusiveList,
}

impl Context {
    fn new() -> Self {
        Context {
            devices: IntrusiveList::new(),
        }
    }
}

static CONTEXT: OnceLock<Context> = OnceLock::new();

/// Init HID service
pub fn init() {
    CONTEXT.get_or_init(Context::new);
}

/// Register a device with the HID service
pub async fn register_device(device: &'static impl DeviceContainer) -> Result<(), intrusive_list::Error> {
    let device = device.get_hid_device();
    CONTEXT.get().await.devices.push(device)?;
    comms::register_endpoint(device, &device.tp).await
}

/// Find a device by its ID
pub async fn get_device(id: DeviceId) -> Option<&'static Device> {
    for device in &CONTEXT.get().await.devices {
        if let Some(data) = device.data::<Device>() {
            if data.id == id {
                return Some(data);
            }
        } else {
            error!("Non-device located in devices list");
        }
    }

    None
}

/// Convenience function to send a request to a HID device
pub async fn send_request(tp: &Endpoint, to: DeviceId, request: Request<'static>) -> Result<(), Infallible> {
    let message = Message {
        id: to,
        data: MessageData::Request(request),
    };
    tp.send(EndpointID::Internal(Internal::Hid), &message).await
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn descriptor_serialize_deserialize() {
        // No particular significance to these values
        let default_regs = RegisterFile::default();
        const HID_VID: u16 = 0x483;
        const HID_PID: u16 = 0x572B;
        const REPORT_DESC_LEN: u16 = 56;
        const INPUT_REPORT_LEN: u16 = 8;
        const OUTPUT_REPORT_LEN: u16 = 45;
        const BCD_VERSION: u16 = 0x0100;
        const VERSION: u16 = 0x0100;

        let descriptor = Descriptor {
            w_hid_desc_length: DESCRIPTOR_LEN as u16,
            bcd_version: BCD_VERSION,
            w_report_desc_length: REPORT_DESC_LEN,
            w_report_desc_register: default_regs.report_desc_reg,
            w_input_register: default_regs.input_reg,
            w_max_input_length: INPUT_REPORT_LEN,
            w_output_register: default_regs.output_reg,
            w_max_output_length: OUTPUT_REPORT_LEN,
            w_command_register: default_regs.command_reg,
            w_data_register: default_regs.data_reg,
            w_vendor_id: HID_VID,
            w_product_id: HID_PID,
            w_version_id: VERSION,
        };

        let mut buf = [0u8; DESCRIPTOR_LEN];
        let _ = descriptor.encode_into_slice(&mut buf).unwrap();
        let decoded = Descriptor::decode_from_slice(&buf).unwrap();

        assert_eq!(decoded, descriptor);
    }
}
