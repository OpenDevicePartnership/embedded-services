//! HID sevices
//! See spec at <http://msdn.microsoft.com/en-us/library/windows/hardware/hh852380.aspx>
use core::convert::Infallible;

use embassy_sync::channel::Channel;

use crate::buffer::SharedRef;
use crate::comms::{self, Endpoint, EndpointID, External, Internal, MailboxDelegate};
use crate::{GlobalRawMutex, IntrusiveList, Node, NodeContainer, error, intrusive_list};

mod command;
pub use command::*;

/// HID descriptor length
pub const DESCRIPTOR_LEN: usize = 30;

/// Data for [`Error::InvalidSize`]
#[derive(Clone, Copy, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct InvalidSizeError {
    /// Expected size
    pub expected: usize,
    /// Actual size
    pub actual: usize,
}

/// HID errors
#[derive(Clone, Copy, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Error {
    /// Invalid data
    InvalidData,
    /// Invalid size: expected and actual sizes
    InvalidSize(InvalidSizeError),
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
    // panic safety: we check the length at the start of the function
    #[allow(clippy::indexing_slicing)]
    pub fn encode_into_slice(&self, buf: &mut [u8]) -> Result<usize, Error> {
        if buf.len() < DESCRIPTOR_LEN {
            return Err(Error::InvalidSize(InvalidSizeError {
                expected: DESCRIPTOR_LEN,
                actual: buf.len(),
            }));
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
    // panic safety: we check the length at the start of the function
    #[allow(clippy::indexing_slicing)]
    pub fn decode_from_slice(buf: &[u8]) -> Result<Self, Error> {
        if buf.len() < DESCRIPTOR_LEN {
            return Err(Error::InvalidSize(InvalidSizeError {
                expected: DESCRIPTOR_LEN,
                actual: buf.len(),
            }));
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

/// Maximum number of in-flight host requests buffered per HID device.
///
/// A depth of 2 allows a single pipelined host request without loss; anything
/// beyond that returns `BufferFull` to the sender so the caller can react
/// instead of losing data silently.
const DEVICE_REQUEST_QUEUE_DEPTH: usize = 2;

/// HID device that responds to HID requests
pub struct Device {
    node: Node,
    tp: Endpoint,
    request: Channel<GlobalRawMutex, Request<'static>, DEVICE_REQUEST_QUEUE_DEPTH>,
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
            request: Channel::new(),
            id,
            regs,
        }
    }

    /// Wait for this device to receive a request
    pub async fn wait_request(&self) -> Request<'static> {
        self.request.receive().await
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

        // All variants must enforce id-matching consistently. Reject mismatched
        // ids uniformly with `InvalidId` rather than silently signaling a
        // request to the wrong device.
        if message.id != self.id {
            return Err(comms::MailboxDelegateError::InvalidId);
        }

        match message.data {
            MessageData::Request(ref request) => {
                // `try_send` returns `BufferFull` instead of silently
                // overwriting a previously-queued request.
                self.request.try_send(request.clone()).map_err(|_| {
                    crate::metrics::hid::bump_request_overflows();
                    comms::MailboxDelegateError::BufferFull
                })
            }
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
    const fn new() -> Self {
        Context {
            devices: IntrusiveList::new(),
        }
    }
}

static CONTEXT: Context = Context::new();

/// Register a device with the HID service.
///
/// Rejects duplicate `DeviceId` registrations: the append-only registry
/// makes it impossible to clean up a successful push, so two devices with
/// the same id would otherwise both receive every routed HID message via
/// `comms::route`, with the non-matching device bumping
/// `comms::delegator_errors` forever. This mirrors the pre-check pattern
/// used by `cfu-service::ClientContext::register_device` and
/// `battery-service::Context::register_fuel_gauge`.
///
/// If the device is pushed into `CONTEXT.devices` but the subsequent
/// `comms::register_endpoint` call fails (or the caller's future is
/// cancelled mid-await), the device remains discoverable via `get_device`
/// but cannot send responses. That partial-state hazard is surfaced by
/// the `metrics::hid::device_registered_without_endpoint` counter; the
/// error is propagated to the caller so it can choose how to react.
pub async fn register_device(device: &'static impl DeviceContainer) -> Result<(), intrusive_list::Error> {
    let device = device.get_hid_device();

    // Pre-check duplicate DeviceId. Catches the most common partial-state
    // scenario at the cheapest place — before any list mutation.
    if get_device(device.id).is_some() {
        return Err(intrusive_list::Error::NodeAlreadyInList);
    }

    CONTEXT.devices.push(device)?;

    // The device is now in the list. If endpoint registration fails or the
    // future is cancelled, we cannot remove it (append-only). Bump the
    // partial-state counter so operators can detect the hazard, then
    // propagate the error to the caller.
    match comms::register_endpoint(device, &device.tp).await {
        Ok(()) => Ok(()),
        Err(err) => {
            crate::metrics::hid::bump_device_registered_without_endpoint();
            crate::warn!("hid: device registered without endpoint - inner registration failed");
            Err(err)
        }
    }
}

/// Find a device by its ID
pub fn get_device(id: DeviceId) -> Option<&'static Device> {
    for device in &CONTEXT.devices {
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
#[allow(clippy::unwrap_used)]
mod test {
    use super::*;

    fn assert_send_sync<T: Send + Sync>() {}

    /// HID `Device` is `NodeContainer` and therefore must be `Send + Sync`.
    /// It composes only `Send + Sync` fields (`Node`, `Endpoint` via its
    /// manual `unsafe impl`, `Channel`, plain ids and registers), so the
    /// auto-derive should hold.
    #[test]
    fn hid_device_is_send_sync() {
        assert_send_sync::<Device>();
    }

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

    /// `Device::receive` must gate every variant on `message.id == self.id`.
    ///
    /// A request whose `message.id` does not match `self.id` must be rejected
    /// with `InvalidId`, matching the behavior of the other arms.
    #[test]
    fn test_device_receive_rejects_request_for_other_device_id() {
        let device = Device::new(DeviceId(1), RegisterFile::default());

        // Construct a request addressed to a different device id (2).
        let request = Request::Descriptor;
        let message = Message {
            id: DeviceId(2),
            data: MessageData::Request(request),
        };
        let envelope = comms::Message {
            from: EndpointID::External(External::Host),
            to: EndpointID::Internal(Internal::Hid),
            data: comms::Data::new(&message),
        };

        let result = device.receive(&envelope);
        assert!(
            matches!(result, Err(comms::MailboxDelegateError::InvalidId)),
            "Request for a different device id must be rejected with InvalidId"
        );
    }

    /// Companion test: request matching the device id must be accepted.
    #[test]
    fn test_device_receive_accepts_request_for_matching_device_id() {
        let device = Device::new(DeviceId(1), RegisterFile::default());

        let request = Request::Descriptor;
        let message = Message {
            id: DeviceId(1),
            data: MessageData::Request(request),
        };
        let envelope = comms::Message {
            from: EndpointID::External(External::Host),
            to: EndpointID::Internal(Internal::Hid),
            data: comms::Data::new(&message),
        };

        let result = device.receive(&envelope);
        assert!(
            matches!(result, Ok(())),
            "Request for matching device id must be accepted"
        );
    }

    /// `Device::receive` must not silently drop back-to-back host requests.
    ///
    /// The device buffers at least one additional pending request without
    /// dropping. The second `receive` must either succeed (buffered) or
    /// return `BufferFull` — never silently overwrite the first.
    #[tokio::test]
    async fn test_device_receive_does_not_silently_drop_back_to_back_requests() {
        use core::time::Duration;

        let device = Device::new(DeviceId(7), RegisterFile::default());

        // Distinct payloads so we can tell them apart on the receive side.
        let req_a = MessageData::Request(Request::Descriptor);
        let req_b = MessageData::Request(Request::ReportDescriptor);

        let msg_a = Message {
            id: DeviceId(7),
            data: req_a,
        };
        let msg_b = Message {
            id: DeviceId(7),
            data: req_b,
        };

        let env_a = comms::Message {
            from: EndpointID::External(External::Host),
            to: EndpointID::Internal(Internal::Hid),
            data: comms::Data::new(&msg_a),
        };
        let env_b = comms::Message {
            from: EndpointID::External(External::Host),
            to: EndpointID::Internal(Internal::Hid),
            data: comms::Data::new(&msg_b),
        };

        // Send first request - must succeed.
        assert!(device.receive(&env_a).is_ok(), "first request must be accepted");

        // Send second request before draining - must NOT silently overwrite.
        // Acceptable outcomes:
        //   - Ok(()): channel buffered both
        //   - Err(BufferFull): explicit signal to caller
        // Pre-fix outcome: Ok(()) but the first request was silently lost.
        let second = device.receive(&env_b);

        // Drain whatever is buffered (use a timeout to avoid hanging if
        // the implementation regressed to single-slot behavior with both stored).
        let first = tokio::time::timeout(Duration::from_millis(100), device.wait_request())
            .await
            .unwrap();

        match second {
            Ok(()) => {
                // Both requests must be retrievable - prove the second one is also there.
                let second_drained = tokio::time::timeout(Duration::from_millis(100), device.wait_request())
                    .await
                    .unwrap();

                // Discriminate the variants - the channel must preserve both, in order.
                assert!(
                    matches!(first, Request::Descriptor),
                    "first delivered must be the first sent (Descriptor)"
                );
                assert!(
                    matches!(second_drained, Request::ReportDescriptor),
                    "second delivered must be the second sent (ReportDescriptor)"
                );
            }
            Err(_) => {
                // Pre-fix path silently dropped on overflow returning Ok(()).
                // If the implementation now returns BufferFull, that's also acceptable.
                // First request must still be the original (Descriptor).
                assert!(
                    matches!(first, Request::Descriptor),
                    "first request must be preserved even when second is rejected"
                );
            }
        }
    }

    /// `Device::receive` must bump the `hid::request_overflows` counter
    /// when the internal channel is full and a request is rejected with
    /// `BufferFull`.
    #[tokio::test]
    async fn test_request_overflow_bumps_counter() {
        let device = Device::new(DeviceId(9), RegisterFile::default());

        let envelope = |req: Request<'static>| {
            let msg = Message {
                id: DeviceId(9),
                data: MessageData::Request(req),
            };
            // The Box keeps the Message alive for the borrow inside Data::new.
            (
                msg,
                EndpointID::External(External::Host),
                EndpointID::Internal(Internal::Hid),
            )
        };

        // Fill the channel to capacity. Capacity is DEVICE_REQUEST_QUEUE_DEPTH
        // which is 2 today, so two `try_send`s must succeed.
        let m1 = envelope(Request::Descriptor);
        let env1 = comms::Message {
            from: m1.1,
            to: m1.2,
            data: comms::Data::new(&m1.0),
        };
        let m2 = envelope(Request::ReportDescriptor);
        let env2 = comms::Message {
            from: m2.1,
            to: m2.2,
            data: comms::Data::new(&m2.0),
        };
        let m3 = envelope(Request::InputReport);
        let env3 = comms::Message {
            from: m3.1,
            to: m3.2,
            data: comms::Data::new(&m3.0),
        };

        assert!(device.receive(&env1).is_ok());
        assert!(device.receive(&env2).is_ok());

        let before = crate::metrics::hid::request_overflows();

        // Third request must overflow and bump the counter.
        let third = device.receive(&env3);
        assert!(
            matches!(third, Err(comms::MailboxDelegateError::BufferFull)),
            "third request must be rejected with BufferFull when channel is full"
        );

        let after = crate::metrics::hid::request_overflows();
        assert!(
            after > before,
            "hid::request_overflows must increase on BufferFull; before={} after={}",
            before,
            after,
        );
    }

    /// `register_device` must reject a second registration that uses the
    /// same `DeviceId`. The append-only registry makes it impossible to
    /// undo a successful push, so the only way to avoid partial state is to
    /// reject the duplicate up front. Mirrors the `cfu-service` and
    /// `battery-service` pre-check pattern.
    #[tokio::test]
    async fn test_register_device_rejects_duplicate_device_id() {
        use embassy_sync::once_lock::OnceLock;

        crate::comms::init();

        // Pick a DeviceId unlikely to collide with other tests that touch
        // the shared CONTEXT.devices list.
        const DUP_ID: DeviceId = DeviceId(0xA1);

        static FIRST: OnceLock<Device> = OnceLock::new();
        let first = FIRST.get_or_init(|| Device::new(DUP_ID, RegisterFile::default()));

        static SECOND: OnceLock<Device> = OnceLock::new();
        let second = SECOND.get_or_init(|| Device::new(DUP_ID, RegisterFile::default()));

        // First registration must succeed.
        register_device(first).await.unwrap();

        // Second registration with the same DeviceId must FAIL before any
        // state is mutated. Without the pre-check, the second `push` would
        // succeed (different Node), the second endpoint would also register
        // (different Endpoint), and every routed HID message would dispatch
        // to both, with the non-matching device bumping
        // `comms::delegator_errors` forever.
        let duplicate = register_device(second).await;
        assert!(duplicate.is_err(), "duplicate DeviceId registration must be rejected");

        // The first device must still be discoverable.
        let found = get_device(DUP_ID);
        assert!(
            found.is_some(),
            "first device must remain in CONTEXT.devices after the rejected duplicate"
        );
    }
}
