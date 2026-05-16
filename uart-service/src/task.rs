use crate::{Error, Service};
use embedded_io_async::Read as UartRead;
use embedded_io_async::Write as UartWrite;
use embedded_services::error;
use embedded_services::relay::mctp::RelayHandler;
use mctp_rs::MctpMedium;

pub async fn uart_service<R: RelayHandler, M: MctpMedium + Copy, T: UartRead + UartWrite>(
    uart_service: &Service<R, M>,
    mut uart: T,
) -> Result<embedded_services::Never, Error<M>> {
    // Note: eSPI service uses `select!` to seemingly allow asyncrhonous `responses` from services,
    // but there are concerns around async cancellation here at least for UART service.
    //
    // Thus this assumes services will only send messages in response to requests from the host,
    // so we handle this in order.
    loop {
        if let Err(e) = uart_service.wait_for_request(&mut uart).await {
            log_request_error(&e);
        } else {
            let host_msg = uart_service.wait_for_response().await;
            if let Err(e) = uart_service.process_response(&mut uart, host_msg).await {
                log_response_error(&e);
            }
        }
    }
}

/// Emit a per-variant static log message for a `wait_for_request` error.
///
/// We can't use `{:?}` because `Error<M>` only implements `defmt::Format`
/// when `M::Error` does too, and that bound can't be expressed as a
/// feature-gated where-clause on stable Rust (rust-lang/rust#115590).
/// Matching on the variant + emitting a fixed string preserves the
/// per-variant signal without that bound.
fn log_request_error<M: MctpMedium>(e: &Error<M>) {
    match e {
        Error::Comms => error!("uart-service request: comms error (uart returned 0 bytes)"),
        Error::Uart => error!("uart-service request: uart I/O error"),
        Error::Mctp(_) => error!("uart-service request: mctp framing/decode error"),
        Error::Serialize(s) => error!("uart-service request: serialize error: {}", s),
        Error::Buffer(_) => error!("uart-service request: buffer error"),
    }
}

/// Emit a per-variant static log message for a `process_response` error.
fn log_response_error<M: MctpMedium>(e: &Error<M>) {
    match e {
        Error::Comms => error!("uart-service response: comms error"),
        Error::Uart => error!("uart-service response: uart I/O error"),
        Error::Mctp(_) => error!("uart-service response: mctp framing/encode error"),
        Error::Serialize(s) => error!("uart-service response: serialize error: {}", s),
        Error::Buffer(_) => error!("uart-service response: buffer error"),
    }
}
