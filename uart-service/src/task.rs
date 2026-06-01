use crate::{Error, ReadState, Service};
use embassy_futures::select::{Either, select};
use embedded_io_async::Read as UartRead;
use embedded_io_async::Write as UartWrite;
use embedded_services::event::Receiver;
use embedded_services::relay::mctp::RelayHandler;
use embedded_services::{error, warn};
use mctp_rs::MctpMedium;

/// Start the UART service task.
///
/// # Requirements
///
/// The `embedded-io-async` `Read` implementation used for the uart **MUST** be cancel-safe.
pub async fn uart_service<R: RelayHandler, M: MctpMedium + Copy, T: UartRead + UartWrite>(
    mut service: Service<R, M>,
    mut uart: T,
) -> Result<embedded_services::Never, Error<M>> {
    let mut read_state = ReadState::new();

    loop {
        match select(
            service.inner.wait_for_request(&mut uart, &mut read_state),
            service.relay_handler.receiver().wait_next(),
        )
        .await
        {
            Either::First(Ok(packet_len)) => {
                if let Err(e) = service.process_request(&read_state, packet_len).await {
                    log_error("request", &e);
                } else {
                    let host_msg = service.inner.wait_for_response().await;
                    if let Err(e) = service.inner.process_response(&mut uart, host_msg).await {
                        log_error("response", &e);
                    }
                }
                read_state.reset();
            }
            Either::First(Err(e)) => {
                log_error("request", &e);
                read_state.reset();
            }
            Either::Second(event) => {
                warn!(
                    "uart-service received notifiable event ({}) from relayable service",
                    event
                );

                // TODO: Here we would do something like:
                //
                // if let Err(_e) = uart.write_all(&[0x42, event]).await {
                //     error!("uart-service failed to send notification");
                // } else {
                //     warn!("uart-service sent notification for event {}", event);
                // }
                //
                // Where we TX some starter byte(s) to tell host it's about to receive a notification,
                // then the notification ID itself.
                //
                // This is TODO until the whole stack is ready to receive notifications
                // otherwise TXing here could break things.
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
