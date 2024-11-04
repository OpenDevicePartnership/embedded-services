use std::borrow::{Borrow, BorrowMut};

use embassy_executor::Executor;
use embassy_sync::once_lock::OnceLock;
use embassy_time::Timer;
use embedded_services::{
    buffer::*,
    define_static_buffer,
    transport::{self, Endpoint, External, Internal},
};
use log::*;
use static_cell::StaticCell;

mod sender {
    use super::*;

    pub struct Sender {
        pub tp: transport::EndpointLink,
        buffer: OwnedRef<'static>,
    }

    impl Sender {
        pub fn new(buffer: OwnedRef<'static>) -> Self {
            Self {
                tp: transport::EndpointLink::uninit(Endpoint::External(External::Host)),
                buffer,
            }
        }

        pub async fn send(&self, even: bool) {
            {
                let mut borrow = self.buffer.borrow_mut();
                let data: &mut [u8] = borrow.borrow_mut();
                let data = &mut data[0..4];
                if even {
                    data.copy_from_slice(&[0, 2, 4, 6]);
                } else {
                    data.copy_from_slice(&[1, 3, 5, 7]);
                }
            }

            self.tp
                .send(Endpoint::Internal(Internal::Oem(0)), &self.buffer.reference())
                .await
                .unwrap();
        }
    }

    impl transport::MessageDelegate for Sender {
        fn process(&self, _message: &transport::Message) {}
    }
}

mod receiver {
    use super::*;

    pub struct Receiver {
        pub tp: transport::EndpointLink,
    }

    impl Receiver {
        pub fn new() -> Self {
            Self {
                tp: transport::EndpointLink::uninit(Endpoint::Internal(Internal::Oem(0))),
            }
        }
    }

    impl transport::MessageDelegate for Receiver {
        fn process(&self, message: &transport::Message) {
            if let Some(data) = message.data.get::<SharedRef>() {
                let borrow = data.borrow();
                let data: &[u8] = borrow.borrow();
                info!("Received data: {:?}", data);
            }
        }
    }
}

#[embassy_executor::task]
async fn task() {
    define_static_buffer!(buffer, [0u8; 8]);
    static SENDER: OnceLock<sender::Sender> = OnceLock::new();
    let sender = SENDER.get_or_init(|| sender::Sender::new(buffer::get_mut().unwrap()));
    static RECEIVER: OnceLock<receiver::Receiver> = OnceLock::new();
    let receiver = RECEIVER.get_or_init(receiver::Receiver::new);
    let mut even = true;

    embedded_services::init().await;

    info!("Registering sender endpoint");
    transport::register_endpoint(sender, &sender.tp).await.unwrap();

    info!("Registering receiver endpoint");
    transport::register_endpoint(receiver, &receiver.tp).await.unwrap();

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
