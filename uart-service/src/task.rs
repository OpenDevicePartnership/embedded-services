use crate::{Error, Service};
use embedded_io_async::Read as UartRead;
use embedded_io_async::Write as UartWrite;
use embedded_services::error;
use mctp_rs::MctpMedium;
use odp_client::server::RelayHandler;

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
            log_error("request", &e);
        } else {
            let host_msg = uart_service.wait_for_response().await;
            if let Err(e) = uart_service.process_response(&mut uart, host_msg).await {
                log_error("response", &e);
            }
        }
    }
}

fn log_error<M: MctpMedium>(direction: &str, e: &Error<M>) {
    match e {
        Error::Comms => error!("uart-service {}: comms error", direction),
        Error::Uart => error!("uart-service {}: uart I/O error", direction),
        Error::Mctp(_) => error!("uart-service {}: mctp error", direction),
        Error::Serialize(s) => error!("uart-service {}: serialize error: {}", direction, s),
        Error::Buffer(_) => error!("uart-service {}: buffer error", direction),
    }
}
