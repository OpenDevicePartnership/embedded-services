use std::borrow::{Borrow, BorrowMut};

use embassy_executor::Executor;
use embassy_sync::once_lock::OnceLock;
use embassy_time::Timer;
use embedded_services::buffer::*;
use embedded_services::comms::{self, EndpointID, External, Internal};
use embedded_services::define_static_buffer;
use log::*;
use static_cell::StaticCell;

mod sender {
    use super::*;

    pub struct Sender {
        pub tp: comms::Endpoint,
        buffer: OwnedRef<'static, u8>,
    }

    impl Sender {
        pub fn new(buffer: OwnedRef<'static, u8>) -> Self {
            Self {
                tp: comms::Endpoint::uninit(EndpointID::External(External::Host)),
                buffer,
            }
        }

        pub async fn send(&self, even: bool) {
            {
                let mut borrow = self.buffer.borrow_mut().unwrap();
                let data: &mut [u8] = borrow.borrow_mut();
                let data = &mut data[0..4];
                if even {
                    data.copy_from_slice(&[0, 2, 4, 6]);
                } else {
                    data.copy_from_slice(&[1, 3, 5, 7]);
                }
            }

            self.tp
                .send(EndpointID::Internal(Internal::Oem(0)), &self.buffer.reference())
                .await
                .unwrap();
        }
    }

    impl comms::MailboxDelegate for Sender {}
}

mod receiver {
    use super::*;

    pub struct Receiver {
        pub tp: comms::Endpoint,
    }

    impl Receiver {
        pub fn new() -> Self {
            Self {
                tp: comms::Endpoint::uninit(EndpointID::Internal(Internal::Oem(0))),
            }
        }
    }

    impl comms::MailboxDelegate for Receiver {
        fn receive(&self, message: &comms::Message) -> Result<(), comms::MailboxDelegateError> {
            let data = message
                .data
                .get::<SharedRef<'_, u8>>()
                .ok_or(comms::MailboxDelegateError::MessageNotFound)?;

            let borrow = data.borrow().unwrap();
            let data: &[u8] = borrow.borrow();
            info!("Received data: {data:?}");

            Ok(())
        }
    }
}

#[embassy_executor::task]
async fn task() {
    define_static_buffer!(buffer, u8, [0; 8]);
    static SENDER: OnceLock<sender::Sender> = OnceLock::new();
    let sender = SENDER.get_or_init(|| sender::Sender::new(buffer::get_mut().unwrap()));
    static RECEIVER: OnceLock<receiver::Receiver> = OnceLock::new();
    let receiver = RECEIVER.get_or_init(receiver::Receiver::new);
    let mut even = true;

    embedded_services::init().await;

    info!("Registering sender endpoint");
    comms::register_endpoint(sender, &sender.tp).await.unwrap();

    info!("Registering receiver endpoint");
    comms::register_endpoint(receiver, &receiver.tp).await.unwrap();

    loop {
        info!("Sending {}", if even { "even" } else { "odd" });
        sender.send(even).await;
        even = !even;
        Timer::after_secs(1).await;
    }
}

fn main() {
    env_logger::builder().filter_level(log::LevelFilter::Info).init();

    static EXECUTOR: StaticCell<Executor> = StaticCell::new();
    let executor = EXECUTOR.init(Executor::new());
    executor.run(|spawner| {
        spawner.spawn(task()).unwrap();
    });
}
