//! HID sevices
//! See spec at http://msdn.microsoft.com/en-us/library/windows/hardware/hh852380.aspx
use crate::{
    buffer::SharedRef,
    error, intrusive_list,
    transport::{self, Endpoint, EndpointLink, External, Internal, MessageDelegate},
    IntrusiveList, Node, NodeContainer,
};
use core::convert::Infallible;
use embassy_sync::{blocking_mutex::raw::NoopRawMutex, once_lock::OnceLock, signal::Signal};

mod command;

pub use command::*;

/// HID descriptor length
pub const DESCRIPTOR_LEN: usize = 30;

/// HID descriptor, see spec for descriptions
#[derive(Clone, Debug, Default)]
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
    /// Writes the descriptor to a slice, returns None if the buffer is not sized correctly
    pub fn write_buffer(&self, buffer: &mut [u8]) -> Option<()> {
        if buffer.len() != DESCRIPTOR_LEN {
            return None;
        }

        buffer[0..2].copy_from_slice(&self.w_hid_desc_length.to_le_bytes());
        buffer[2..4].copy_from_slice(&self.bcd_version.to_le_bytes());
        buffer[4..6].copy_from_slice(&self.w_report_desc_length.to_le_bytes());
        buffer[6..8].copy_from_slice(&self.w_report_desc_register.to_le_bytes());
        buffer[8..10].copy_from_slice(&self.w_input_register.to_le_bytes());
        buffer[10..12].copy_from_slice(&self.w_max_input_length.to_le_bytes());
        buffer[12..14].copy_from_slice(&self.w_output_register.to_le_bytes());
        buffer[14..16].copy_from_slice(&self.w_max_output_length.to_le_bytes());
        buffer[16..18].copy_from_slice(&self.w_command_register.to_le_bytes());
        buffer[18..20].copy_from_slice(&self.w_data_register.to_le_bytes());
        buffer[20..22].copy_from_slice(&self.w_vendor_id.to_le_bytes());
        buffer[22..24].copy_from_slice(&self.w_product_id.to_le_bytes());
        buffer[24..26].copy_from_slice(&self.w_version_id.to_le_bytes());
        // Last four bytes are reserved
        buffer[26..30].fill(0);
        Some(())
    }
}

/// HID device
pub struct Device {
    node: Node,
    tp: EndpointLink,
    request: Signal<NoopRawMutex, Request<'static>>,
    /// Device ID
    pub id: DeviceId,
    /// HID descriptor register
    pub hid_desc_register: u16,
    /// HID report descriptor register
    pub hid_report_desc_register: u16,
    /// HID input report register
    pub hid_input_register: u16,
    /// HID output report register
    pub hid_output_register: u16,
    /// HID command register
    pub hid_command_register: u16,
    /// HID data register
    pub hid_data_register: u16,
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
    pub fn new(
        id: DeviceId,
        hid_desc_register: u16,
        hid_report_desc_register: u16,
        hid_input_register: u16,
        hid_output_register: u16,
        hid_command_register: u16,
        hid_data_register: u16,
    ) -> Self {
        Self {
            node: Node::uninit(),
            tp: EndpointLink::uninit(Endpoint::Internal(Internal::Hid)),
            request: Signal::new(),
            id,
            hid_desc_register,
            hid_report_desc_register,
            hid_input_register,
            hid_output_register,
            hid_command_register,
            hid_data_register,
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
        self.tp.send(Endpoint::External(External::Host), &message).await
    }
}

impl MessageDelegate for Device {
    fn process(&self, message: &transport::Message) {
        if let Some(message) = message.data.get::<Message>() {
            if message.id != self.id {
                return;
            }

            if let MessageData::Request(ref request) = message.data {
                self.request.signal(request.clone());
            }
        }
    }
}

/// HID device ID
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct DeviceId(pub u8);

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
    OutputReport(Option<ReportId>, SharedRef<'a>),
    /// Command
    Command(Command<'a>),
}

/// Device to host messages
#[derive(Clone)]
pub enum Response<'a> {
    /// HID descriptor response
    /// none indicates that the HID service should use HID descriptor stored in the device
    Descriptor(SharedRef<'a>),
    /// Report descriptor response
    ReportDescriptor(SharedRef<'a>),
    /// Input report
    InputReport(SharedRef<'a>),
    /// Feature report
    FeatureReport(SharedRef<'a>),
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
    transport::register_endpoint(device, &device.tp).await
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
pub async fn send_request(tp: &EndpointLink, to: DeviceId, request: Request<'static>) -> Result<(), Infallible> {
    let message = Message {
        id: to,
        data: MessageData::Request(request),
    };
    tp.send(Endpoint::Internal(Internal::Hid), &message).await
}
