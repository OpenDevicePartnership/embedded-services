use crate::{Error, Service};
use embedded_io_async::Read as UartRead;
use embedded_io_async::Write as UartWrite;
use embedded_services::comms;
use embedded_services::error;

pub async fn uart_service<T: UartRead + UartWrite>(
    uart_service: &'static Service<'_>,
    mut uart: T,
) -> Result<embedded_services::Never, Error> {
    // Register uart-service as the host service
    comms::register_endpoint(uart_service, &uart_service.endpoint)
        .await
        .map_err(|_| Error::Comms)?;

    // Note: eSPI service uses `select!` to seemingly allow asyncrhonous `responses` from services,
    // but there are concerns around async cancellation here at least for UART service.
    //
    // Thus this assumes services will only send messages in response to requests from the host,
    // so we handle this in order.
    loop {
        if let Err(e) = uart_service.wait_for_request(&mut uart).await {
            error!("uart-service request error: {:?}", e);
        } else {
            let host_msg = uart_service.wait_for_response().await;
            if let Err(e) = uart_service.process_response(&mut uart, &host_msg).await {
                error!("uart-service response error: {:?}", e)
            }
        }
    }
}
