//! Handles the backend HID communication with host for the keyboard
use super::HidKeyboard;
use core::borrow::BorrowMut;
use embassy_sync::channel::Channel;
use embassy_sync::once_lock::OnceLock;
use embassy_sync::signal::Signal;
use embedded_hal::digital::OutputPin;
use embedded_services::GlobalRawMutex;
use embedded_services::buffer::SharedRef;
use embedded_services::comms;
use embedded_services::error;
use embedded_services::hid;
use embedded_services::ipc::deferred as ipc;
use hid_service::i2c::I2cSlaveAsync;
use static_cell::StaticCell;

// Revisit: Figure out the best way to make these caller configurable
// According to spec input reports can be upto u16 max, but we don't want a queue
// with 65k bytes * queue size, so need to investigate smarter way of supporting theoretical max
// efficiently.
const INPUT_MAX: usize = 16;
const REPORT_DESC_MAX: usize = 256;
const REPORT_QUEUE_MAX: usize = 10;

// The size of a HID i2c report header
pub const I2C_REPORT_HEADER_SZ: usize = REPORT_LEN_SZ + REPORT_ID_SZ;

// Max size of a HID report not including i2c header
const REPORT_MAX_SZ: usize = INPUT_MAX - I2C_REPORT_HEADER_SZ;

// I2C input reports begin with 2 byte length of the report
const REPORT_LEN_SZ: usize = 2;

// A input report
const REPORT_ID: u8 = 1;

// Indicates the report ID (a single device like a keyboard might have multiple report types)
// If only a single type, can be omitted. But include it anyway for future-proofing.
const REPORT_ID_SZ: usize = 1;

type Report = [u8; INPUT_MAX];
type ReportQueue = Channel<GlobalRawMutex, Report, REPORT_QUEUE_MAX>;
type CmdIpc = ipc::Channel<GlobalRawMutex, hid::Command<'static>, Option<hid::Response<'static>>>;
type ReportIpc = ipc::Channel<GlobalRawMutex, SharedRef<'static, u8>, ()>;

// A HID input report in the format HID over i2c expects
#[derive(Default)]
struct HidI2cReport([u8; INPUT_MAX]);

impl HidI2cReport {
    // Conenience for the raw bytes
    fn to_bytes(&self) -> [u8; INPUT_MAX] {
        self.0
    }

    fn from_report_slice(report: super::HidReportSlice, max_len: u16) -> Self {
        let mut buf = Self::default().0;
        let bytes = report.as_bytes();

        // Report length
        // This always needs to be set to the max_input_len field from the HID descriptor
        // Why the need for this redundancy? Who knows.
        buf[0..REPORT_LEN_SZ].copy_from_slice(&max_len.to_le_bytes());

        // Report type/id
        buf[2] = REPORT_ID;

        // Modifer keys byte and usage codes
        //
        // Revisit: Discards bytes from the report greater than we can handle
        // Not great, so will change once I have a better idea of allowing
        // max buffer sizes to be configurable
        let len = bytes.len().min(REPORT_MAX_SZ);
        buf[3..3 + len].copy_from_slice(&bytes[..len]);

        Self(buf)
    }

    fn from_error(error: super::KeyboardError, max_len: u16) -> Self {
        const ERROR_ROLL_OVER: u8 = 0x01;
        const ERROR_UNDEFINED: u8 = 0x03;

        let err = match error {
            super::KeyboardError::Ghosting | super::KeyboardError::Rollover => [ERROR_ROLL_OVER; REPORT_MAX_SZ],
            super::KeyboardError::Scan | super::KeyboardError::Command => [ERROR_UNDEFINED; REPORT_MAX_SZ],
        };

        HidI2cReport::from_report_slice(super::HidReportSlice(&err), max_len)
    }
}

// Shared between tasks for communication and synchronization
struct Context {
    report_queue: ReportQueue,
    report_ipc: ReportIpc,
    cmd_ipc: CmdIpc,
    send_complete: Signal<GlobalRawMutex, ()>,
}
static CONTEXT: OnceLock<Context> = OnceLock::new();

// Sets up the context, report descriptor buffer, and HID device
pub(crate) async fn init(
    spawner: embassy_executor::Spawner,
    hid_descriptor: hid::Descriptor,
    report_descriptor: &'static [u8],
    reg_file: hid::RegisterFile,
) {
    // Initialize interprocess comms/synchronization context
    let context = Context {
        report_queue: ReportQueue::new(),
        report_ipc: ReportIpc::new(),
        cmd_ipc: CmdIpc::new(),
        send_complete: Signal::new(),
    };
    CONTEXT
        .init(context)
        .map_err(|_| ())
        .expect("Keyboard service already initialized");

    // Initialize the HID device
    static DEVICE: StaticCell<hid::Device> = StaticCell::new();
    let device = hid::Device::new(super::HID_KB_ID, reg_file);
    let device = DEVICE.init(device);
    hid::register_device(device)
        .await
        .expect("Device must not already be registered");

    // Spawn device request handling task
    // Other tasks are spawned by user due to need for macro to implement them because of generics
    spawner.must_spawn(device_requests_task(device, hid_descriptor, report_descriptor));
}

// This task handles receiving HID requests from the host,
// forwarding them to the keyboard task to process, then sending a response back to host
#[embassy_executor::task]
async fn device_requests_task(
    device: &'static hid::Device,
    hid_descriptor: hid::Descriptor,
    report_descriptor: &'static [u8],
) {
    let context = CONTEXT.get().await;

    // Buffer holding hid descriptor
    embedded_services::define_static_buffer!(hid_desc_buf, u8, [0u8; hid::DESCRIPTOR_LEN]);
    {
        let mut buf = hid_desc_buf::get_mut()
            .expect("Must not already be borrowed mutably")
            .borrow_mut();
        let buf: &mut [u8] = buf.borrow_mut();
        hid_descriptor
            .encode_into_slice(buf)
            .expect("Src and dst buffers must be same length");
    }

    // Buffer holding report descriptor
    embedded_services::define_static_buffer!(report_desc_buf, u8, [0u8; REPORT_DESC_MAX]);
    {
        let mut buf = report_desc_buf::get_mut()
            .expect("Must not already be borrowed mutably")
            .borrow_mut();
        let buf: &mut [u8] = buf.borrow_mut();
        buf[..report_descriptor.len()].copy_from_slice(report_descriptor);
    }

    loop {
        let request = device.wait_request().await;
        match request {
            // For descriptors, we simply pass references to respective buffers
            // These are static and never change, so don't need to do much else
            hid::Request::Descriptor => {
                let response = hid_desc_buf::get();
                let response = Some(hid::Response::Descriptor(response));
                device.send_response(response).await.expect("Infallible");
            }
            hid::Request::ReportDescriptor => {
                let response = report_desc_buf::get().slice(0..report_descriptor.len());
                let response = Some(hid::Response::ReportDescriptor(response));
                device.send_response(response).await.expect("Infallible");
            }

            // We won't receive this request unless keyboard told host we have reports available (via interrupt assert)
            hid::Request::InputReport => {
                // Wait for the keyboard to give us the report
                let ipc = context.report_ipc.receive().await;
                let report = ipc.command.clone();
                let response = Some(hid::Response::InputReport(
                    report.slice(0..hid_descriptor.w_max_input_length as usize),
                ));

                // Then send it to the host
                device.send_response(response).await.expect("Infallible");

                // Finally tell keyboard we've sent the report so it can deassert interrupt
                ipc.respond(());
            }

            // Treat this as a SET_REPORT(Output) command
            // It is unclear if the behavior is meant to be different, or just different ways
            // of transporting the same request.
            hid::Request::OutputReport(id, buf) => {
                let response = context
                    .cmd_ipc
                    .execute(hid::Command::SetReport(
                        hid::ReportType::Output,
                        id.unwrap_or(hid::ReportId(1)),
                        buf,
                    ))
                    .await;
                device.send_response(response).await.expect("Infallible");
            }

            // Tell the keyboard to execute the requested command, waiting for it to give us a response to send to host
            hid::Request::Command(cmd) => {
                let response = context.cmd_ipc.execute(cmd).await;
                device.send_response(response).await.expect("Infallible");
            }
        }
    }
}

/// This task handles calling the keyboard `scan` in a loop, while also listening for commands
/// from the HID request handler task. To minimize delay between scan loops, we quickly process commands
/// and let the HID request handler task handle forwarding the response to the host.
pub async fn handle_keyboard<T: HidKeyboard>(mut hid_kb: T) {
    let context = CONTEXT.get().await;

    // Buffer holding immediate report requests
    embedded_services::define_static_buffer!(report_buf, u8, [0u8; INPUT_MAX]);
    let owned_buf = report_buf::get_mut().expect("Must not already be borrowed mutably");
    let max_input_len = hid_kb.hid_descriptor().w_max_input_length;

    loop {
        // Wait for either a command request or input report to become available
        match embassy_futures::select::select(hid_kb.scan(), context.cmd_ipc.receive()).await {
            // If we got a keyboard report, queue it up to be sent out
            embassy_futures::select::Either::First(report) => {
                let i2c_report = match report {
                    Ok(report) => {
                        // Revisit: Look into ways to avoid multiple copies (even if reports are small)
                        // But, difficult to store slices/references in queue with all the lifetime management that entails
                        // May need some form of ring buffer if really need to squeeze performance?
                        HidI2cReport::from_report_slice(report, max_input_len).to_bytes()
                    }
                    Err(e) => HidI2cReport::from_error(e, max_input_len).to_bytes(),
                };

                context.report_queue.send(i2c_report).await;
            }

            // Otherwise if we are instructed to perform a command, do it quickly then respond
            // Revisit: For commands that are fallible, realistically what can we do other than print an error?
            embassy_futures::select::Either::Second(request) => match request.command {
                // A reset is handled similarly to an input report.
                // When we receive a reset command, we must place reset sentinel value ([0x00, 0x00])
                // into report buffer, then assert interrupt so host can read it after we've reset the keyboard.
                hid::Command::Reset => {
                    if hid_kb.reset().await.is_ok() {
                        // Spec says device should enter power on state after reset
                        if hid_kb.set_power_state(hid::PowerState::On).await.is_err() {
                            error!("Failed to set keyboard powerstate to ON");
                        }

                        context.report_queue.send([0x00; INPUT_MAX]).await;
                        request.respond(None);
                    } else {
                        error!("Failed to reset keyboard");
                    }
                }

                // Instructs the keyboard to immediately return the latest input/feature report
                hid::Command::GetReport(report_type, report_id) => {
                    {
                        let report = hid_kb.get_report(report_type, report_id);
                        let report = HidI2cReport::from_report_slice(report, max_input_len).to_bytes();
                        let mut buf = owned_buf.borrow_mut();
                        let buf: &mut [u8] = buf.borrow_mut();
                        buf[..report.len()].copy_from_slice(&report);
                    }
                    request.respond(Some(hid::Response::InputReport(report_buf::get())));
                }

                // Instructs the keyboard to immediately set the output/feature report
                hid::Command::SetReport(report_type, report_id, ref buf) => {
                    if hid_kb.set_report(report_type, report_id, buf).await.is_ok() {
                        request.respond(None);
                    } else {
                        error!("Failed to set keyboard report");
                    }
                }

                // Gets the keyboard's idle time before sending a report even if no changes
                // Not typically used by modern hosts, but we support it anyway
                hid::Command::GetIdle(report_id) => {
                    let freq = hid_kb.get_idle(report_id);
                    request.respond(Some(hid::Response::Command(hid::CommandResponse::GetIdle(freq))));
                }

                // Sets the keyboard's idle time before sending a report even if no changes
                // Not typically used by modern hosts, but we support it anyway
                hid::Command::SetIdle(report_id, report_freq) => {
                    if hid_kb.set_idle(report_id, report_freq).await.is_ok() {
                        request.respond(None);
                    } else {
                        error!("Failed to set keyboard idle");
                    }
                }

                // Gets the keyboard protocol (Boot or Report)
                hid::Command::GetProtocol => {
                    let protocol = hid_kb.get_protocol();
                    request.respond(Some(hid::Response::Command(hid::CommandResponse::GetProtocol(
                        protocol,
                    ))));
                }

                // Sets the keyboard protocol (Boot or Report)
                hid::Command::SetProtocol(protocol) => {
                    if hid_kb.set_protocol(protocol).await.is_ok() {
                        request.respond(None);
                    } else {
                        error!("Failed to set keyboard protocol");
                    }
                }

                // Sets the power state of the keyboard (On or Sleep)
                hid::Command::SetPower(power_state) => {
                    if hid_kb.set_power_state(power_state).await.is_ok() {
                        request.respond(None);
                    } else {
                        error!("Failed to set keyboard power state");
                    }
                }

                // Vendor defined command
                hid::Command::Vendor => {
                    if hid_kb.vendor_cmd().await.is_ok() {
                        request.respond(None);
                    } else {
                        error!("Failed to execute vendor keyboard command");
                    }
                }
            },
        }
    }
}

/// This task handles queueing up input reports as they are generated, asserting interrupts to the host,
/// and synchronizing with the device request handler to ensure they are sent to the host properly.
///
/// This is a separate task because we want the main `scan` loop to quickly fire off an available report
/// without it being blocked waiting for communication with the host. We also use a queue in case multiple reports
/// are available before one is fully processed to prevent any lost key events.
pub async fn handle_reports(mut kb_int: impl OutputPin) {
    let context = CONTEXT.get().await;

    embedded_services::define_static_buffer!(input_buf, u8, [0u8; INPUT_MAX]);
    let owned_buf = input_buf::get_mut().expect("Must not already be borrowed immutably");

    loop {
        // Wait for keyboard to push a report to the queue
        let report = context.report_queue.receive().await;

        // Wait for previous sends to complete
        // Necessary since `handle_host_requests` is borrowing input_buf during duration of send
        context.send_complete.wait().await;

        // Once we have one, copy it to outgoing buffer
        {
            let mut buf = owned_buf.borrow_mut();
            let buf: &mut [u8] = buf.borrow_mut();
            buf.copy_from_slice(&report);
        }

        // Then assert interrupt so host knows to send us a read command
        if kb_int.set_low().is_err() {
            error!("Failed to set keyboard interrupt pin low! Canceling report.");
            continue;
        }

        // Send the buffer reference to request handler, waiting for it to tell us it finished sending the report
        context.report_ipc.execute(input_buf::get()).await;

        // Finally deassert interrupt
        if kb_int.set_high().is_err() {
            error!("Failed to set keyboard interrupt pin high! Host may not respond properly.");
        }
    }
}

/// This task handles listening for raw i2c commands from the host, detecting what kind of request it is,
/// then forwarding that request to the device request listener.
pub async fn handle_host_requests(host: &'static mut hid_service::i2c::Host<impl I2cSlaveAsync>) {
    let context = CONTEXT.get().await;

    comms::register_endpoint(host, &host.tp)
        .await
        .expect("Host must not already be registered.");

    loop {
        let res = host.process().await;
        match res {
            Ok(()) => context.send_complete.signal(()),
            Err(hid_service::Error::Bus(_)) => error!("Host I2C bus error"),
            Err(hid_service::Error::Hid(e)) => error!("Host HID error: {:?}", e),
        }
    }
}
