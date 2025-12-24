use core::borrow::BorrowMut;

use embedded_services::hid;

use crate::hid_kb::{self, CONTEXT};

pub async fn keyboard_task<T: crate::HidKeyboard>(
    keyboard: T,
) -> Result<embedded_services::Never, super::KeyboardError> {
    crate::hid_kb::handle_keyboard(keyboard).await
}

pub async fn reports_task<T: embedded_hal::digital::OutputPin>(
    keyboard_interrupt: T,
) -> Result<embedded_services::Never, super::KeyboardError> {
    crate::hid_kb::handle_reports(keyboard_interrupt).await
}

// Since Rust doesn't allow defining an inner item that captures generics from an outer item,
// this must be a macro until statics are removed.
#[macro_export]
macro_rules! impl_host_request_task {
    ($i2c_slave_ty:ty) => {
        async fn host_requests_task(kb_i2c: $i2c_slave_ty) {
            // Revisit: Make this buffer size configurable?
            embedded_services::define_static_buffer!(hid_buf, u8, [0u8; 256]);
            let buf = hid_buf::get_mut().expect("Must not already be borrowed mutably");

            // In this macro since static items cannot be generic either
            static HOST: ::static_cell::StaticCell<hid_service::i2c::Host<$i2c_slave_ty>> =
                ::static_cell::StaticCell::new();
            let host = hid_service::i2c::Host::new(keyboard_service::HID_KB_ID, kb_i2c, buf);
            let host = HOST.init(host);

            keyboard_service::hid_kb::handle_host_requests(host).await;
        }
    };
}

/// Initialize the keyboard service given keyboard's HID configuration.
///
/// The user must also ensure the `impl_host_request_task!` macro is called to implement an additional
/// task and then manually spawn it. E.g.:
///
/// ```rust,ignore
/// impl_host_request_task!(MyI2cSlaveType);
/// spawner.must_spawn(host_requests_task(my_i2c_slave));
/// ```
// This task handles receiving HID requests from the host,
// forwarding them to the keyboard task to process, then sending a response back to host
pub async fn init_and_recv_device_requests_task(
    hid_descriptor: hid::Descriptor,
    report_descriptor: &'static [u8],
    reg_file: hid::RegisterFile,
) -> Result<embedded_services::Never, super::KeyboardError> {
    let device = crate::hid_kb::init(reg_file);
    hid::register_device(device)
        .await
        .expect("Device must not already be registered");
    let context = CONTEXT.get().await;

    // Buffer holding hid descriptor
    embedded_services::define_static_buffer!(hid_desc_buf, u8, [0u8; hid::DESCRIPTOR_LEN]);
    {
        let mut buf = hid_desc_buf::get_mut()
            .expect("Must not already be borrowed mutably")
            .borrow_mut()
            .map_err(super::KeyboardError::Buffer)?;
        let buf: &mut [u8] = buf.borrow_mut();
        hid_descriptor
            .encode_into_slice(buf)
            .expect("Src and dst buffers must be same length");
    }

    // Buffer holding report descriptor
    embedded_services::define_static_buffer!(report_desc_buf, u8, [0u8; hid_kb::REPORT_DESC_MAX]);
    {
        let mut buf = report_desc_buf::get_mut()
            .expect("Must not already be borrowed mutably")
            .borrow_mut()
            .map_err(super::KeyboardError::Buffer)?;
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
                let response = report_desc_buf::get()
                    .slice(0..report_descriptor.len())
                    .map_err(super::KeyboardError::Buffer)?;
                let response = Some(hid::Response::ReportDescriptor(response));
                device.send_response(response).await.expect("Infallible");
            }

            // We won't receive this request unless keyboard told host we have reports available (via interrupt assert)
            hid::Request::InputReport => {
                // Wait for the keyboard to give us the report
                let ipc = context.report_ipc.receive().await;
                let report = ipc.command.clone();
                let response = Some(hid::Response::InputReport(
                    report
                        .slice(0..hid_descriptor.w_max_input_length as usize)
                        .map_err(super::KeyboardError::Buffer)?,
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
