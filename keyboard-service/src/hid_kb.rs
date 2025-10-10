//! Handles the backend HID communication with host for the keyboard
use super::HidKeyboard;
use core::borrow::BorrowMut;
use embassy_sync::channel::Channel;
use embassy_sync::once_lock::OnceLock;
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

type Report = [u8; INPUT_MAX];
type ReportQueue = Channel<GlobalRawMutex, Report, REPORT_QUEUE_MAX>;
type CmdIpc = ipc::Channel<GlobalRawMutex, hid::Command<'static>, Option<hid::Response<'static>>>;
type ReportIpc = ipc::Channel<GlobalRawMutex, SharedRef<'static, u8>, ()>;

// Shared between tasks for communication and synchronization
struct Context {
    report_queue: ReportQueue,
    report_ipc: ReportIpc,
    cmd_ipc: CmdIpc,
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

    // Spawn device requets handling task
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
pub async fn handle_keyboard(mut hid_kb: impl HidKeyboard) {
    let context = CONTEXT.get().await;

    // Buffer holding immediate report requests
    embedded_services::define_static_buffer!(report_buf, u8, [0u8; INPUT_MAX]);
    let owned_buf = report_buf::get_mut().expect("Must not already be borrowed mutably");

    loop {
        // Wait for either a command request or input report to become available
        match embassy_futures::select::select(hid_kb.scan(), context.cmd_ipc.receive()).await {
            // If we got a keyboard report, queue it up the to be sent out
            embassy_futures::select::Either::First(report) => {
                // Revisit: Look into ways to avoid multiple copies (even if reports are small)
                // But, difficult to store slices/references in queue with all the lifetime management that entails
                // May need some form of ring buffer if really need to squeeze performance?
                let mut report_buf = [0x00; INPUT_MAX];
                report_buf[..report.len()].copy_from_slice(report);
                context.report_queue.send(report_buf).await;
            }

            // Otherwise if we are instructed to perform a command, do it quickly then respond
            embassy_futures::select::Either::Second(request) => match request.command {
                // A reset is handled similarly to an input report.
                // When we receive a reset command, we must place reset sentinel value ([0x00, 0x00])
                // into report buffer, then assert interrupt so host can read it after we've reset the keyboard.
                hid::Command::Reset => {
                    hid_kb.reset().await;
                    // Spec says device should enter power on state after reset
                    hid_kb.set_power_state(hid::PowerState::On).await;
                    context.report_queue.send([0x00; INPUT_MAX]).await;
                    request.respond(None);
                }

                // Instructs the keyboard to immediately return the latest input/feature report
                hid::Command::GetReport(report_type, report_id) => {
                    let report = hid_kb.get_report(report_type, report_id).await;
                    let mut buf = owned_buf.borrow_mut();
                    let buf: &mut [u8] = buf.borrow_mut();
                    buf[..report.len()].copy_from_slice(report);
                    request.respond(Some(hid::Response::InputReport(report_buf::get())));
                }

                // Instructs the keyboard to immedaitely set the output/feature report
                hid::Command::SetReport(report_type, report_id, ref buf) => {
                    hid_kb.set_report(report_type, report_id, buf).await;
                    request.respond(None);
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
                    hid_kb.set_idle(report_id, report_freq).await;
                    request.respond(None);
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
                    hid_kb.set_protocol(protocol).await;
                    request.respond(None);
                }

                // Sets the power state of the keyboard (On or Sleep)
                hid::Command::SetPower(power_state) => {
                    hid_kb.set_power_state(power_state).await;
                    request.respond(None);
                }

                // Vendor defined command
                hid::Command::Vendor => {
                    hid_kb.vendor_cmd().await;
                    request.respond(None);
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
    comms::register_endpoint(host, &host.tp)
        .await
        .expect("Host must not already be registered.");

    loop {
        let res = host.process().await;
        match res {
            Ok(()) => (),
            Err(hid_service::Error::Bus(_)) => error!("Host I2C bus error"),
            Err(hid_service::Error::Hid(e)) => error!("Host HID error: {:?}", e),
        }
    }
}
