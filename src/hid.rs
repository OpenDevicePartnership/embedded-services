//! HID sevices
use embassy_sync::once_lock::OnceLock;

use crate::{
    transport::{Endpoint, EndpointLink, Internal},
    IntrusiveList, Node, NodeContainer,
};

/// HID descriptor, see spec for descriptions
#[derive(Clone, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[allow(missing_docs)]
pub struct HidDescriptor {
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
    pub _reserved: [u8; 4],
}

/// HID device
pub struct Device {
    node: Node,
    /// Device ID
    pub id: DeviceId,
    /// HID descriptor read register
    pub hid_desc_reg: u16,
    /// HID descriptor
    pub hid_desc: &'static HidDescriptor,
}

impl NodeContainer for Device {
    fn get_node(&self) -> &Node {
        &self.node
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

/// HID report types
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum ReportType {
    /// Reserved
    Reserved,
    /// Input report
    Input,
    /// Output report
    Output,
    /// Feature report
    Feature,
}

/// HID command op codes, see spec for descriptions
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[allow(missing_docs)]
pub enum CommandOp {
    Reserved,
    Reset,
    GetReport,
    SetReport,
    GetIdle,
    SetIdle,
    GetProtocol,
    SetProtocol,
    SetPower,
    Vendor,
}

/// Host to device messages
#[derive(Clone, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Request {
    /// HID descriptor request
    HidDescriptor,
    /// Report descriptor request
    ReportDescriptor,
    /// Input report request
    InputReport,
    /// Output report request
    OutputReport(Option<ReportId>, &'static [u8]),
    /// Command
    Command(CommandOp, ReportType, ReportId, Option<&'static [u8]>),
}

/// Device to host messages
#[derive(Clone, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Response {
    /// HID descriptor response
    /// none indicates that the HID service should use HID descriptor stored in the device
    HidDescriptor(Option<&'static HidDescriptor>),
    /// Report descriptor response
    ReportDescriptor(&'static [u8]),
    /// Input report
    InputReport(Option<ReportId>, &'static [u8]),
    /// Generic response for command requests
    Command(&'static [u8]),
}

/// HID message data
#[derive(Clone, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum MessageData {
    /// HID read/write request to register
    Request(Request),
    /// HID request
    Response(Response),
}

/// Top-level struct for HID communication
#[derive(Clone, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Message {
    /// Target/originating device ID
    pub id: DeviceId,
    /// Message contents
    pub data: MessageData,
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

/// Find a device by its ID
pub async fn get_device(id: DeviceId) -> Option<&'static Device> {
    for device in &CONTEXT.get().await.devices {
        let data = device.data::<Device>().unwrap();
        if data.id == id {
            return Some(data);
        }
    }

    None
}

/// Convenience function to send a request to a HID device
pub async fn send_request(tp: &EndpointLink, to: DeviceId, request: Request) {
    let message = Message {
        id: to,
        data: MessageData::Request(request.clone()),
    };
    tp.send(Endpoint::Internal(Internal::Hid), &message).await.unwrap();
}

/// Convenience function to send a response from a HID device
pub async fn send_response(tp: &EndpointLink, from: DeviceId, response: Response) {
    let message = Message {
        id: from,
        data: MessageData::Response(response.clone()),
    };
    tp.send(Endpoint::Internal(Internal::Hid), &message).await.unwrap();
}
