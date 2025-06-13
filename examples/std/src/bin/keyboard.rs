use embassy_executor::{Executor, Spawner};
use embassy_sync::once_lock::OnceLock;
use embassy_time::Timer;
use embedded_services::keyboard::{self, DeviceId, Key, KeyEvent};
use embedded_services::{comms, define_static_buffer};
use log::info;
use static_cell::StaticCell;

/// Mock that generates keyboard messages
mod device {
    use std::borrow::BorrowMut;

    use embedded_services::buffer::OwnedRef;
    use embedded_services::keyboard::{self, DeviceId, Event, Key, KeyEvent, MessageData};

    pub struct Device {
        id: DeviceId,
        event_buffer: OwnedRef<'static, KeyEvent>,
    }

    impl Device {
        pub fn new(id: DeviceId, event_buffer: OwnedRef<'static, KeyEvent>) -> Self {
            Self { id, event_buffer }
        }

        pub async fn key_down(&self, key: Key) {
            {
                let mut borrow = self.event_buffer.borrow_mut();
                let buf: &mut [KeyEvent] = borrow.borrow_mut();

                buf[0] = KeyEvent::Make(key);
            }

            keyboard::broadcast_message(
                self.id,
                MessageData::Event(Event::KeyEvent(self.id, self.event_buffer.reference().slice(0..1))),
            )
            .await;
        }

        pub async fn key_up(&self, key: Key) {
            {
                let mut borrow = self.event_buffer.borrow_mut();
                let buf: &mut [KeyEvent] = borrow.borrow_mut();

                buf[0] = KeyEvent::Break(key);
            }

            keyboard::broadcast_message(
                self.id,
                MessageData::Event(Event::KeyEvent(self.id, self.event_buffer.reference().slice(0..1))),
            )
            .await;
        }
    }
}

/// Mock host device
mod host {
    use std::borrow::Borrow;

    use embedded_services::comms::{self, Endpoint, EndpointID, External, MailboxDelegate};
    use embedded_services::keyboard::{Event, KeyEvent, Message, MessageData};
    use log::info;

    pub struct Host {
        pub tp: Endpoint,
    }

    impl Host {
        pub fn new() -> Self {
            Self {
                tp: Endpoint::uninit(EndpointID::External(External::Host)),
            }
        }
    }

    impl MailboxDelegate for Host {
        fn receive(&self, message: &comms::Message) -> Result<(), comms::MailboxDelegateError> {
            let message = message
                .data
                .get::<Message>()
                .ok_or(comms::MailboxDelegateError::MessageNotFound)?;

            match &message.data {
                MessageData::Event(Event::KeyEvent(id, events)) => {
                    let borrow = events.borrow();
                    let buf: &[KeyEvent] = borrow.borrow();

                    for event in buf {
                        info!("Host received event from device {}: {:?}", id.0, event);
                    }

                    Ok(())
                }
            }
        }
    }
}

const DEVICE0_ID: DeviceId = DeviceId(0);

#[embassy_executor::task]
async fn device() {
    define_static_buffer!(buffer, KeyEvent, [KeyEvent::Break(Key(0)); 8]);

    info!("Device task");
    static DEVICE0: OnceLock<device::Device> = OnceLock::new();
    let this = DEVICE0.get_or_init(|| device::Device::new(DEVICE0_ID, buffer::get_mut().unwrap()));
    info!("Registering device 0 endpoint");

    loop {
        info!("Sending message");
        this.key_down(Key(0x04)).await;
        Timer::after_millis(250).await;
        this.key_up(Key(0x04)).await;
        Timer::after_secs(1).await;
    }
}

#[embassy_executor::task]
async fn host() {
    info!("Host task");
    static HOST: OnceLock<host::Host> = OnceLock::new();
    let this = HOST.get_or_init(host::Host::new);
    info!("Registering host endpoint");
    comms::register_endpoint(this, &this.tp).await.unwrap();
}

#[embassy_executor::task]
async fn run(spawner: Spawner) {
    embedded_services::init().await;

    keyboard::enable_broadcast_host().await;

    spawner.must_spawn(host());
    spawner.must_spawn(device());
}

fn main() {
    env_logger::builder().filter_level(log::LevelFilter::Info).init();

    static EXECUTOR: StaticCell<Executor> = StaticCell::new();
    let executor = EXECUTOR.init(Executor::new());
    executor.run(|spawner| {
        spawner.must_spawn(run(spawner));
    });
}
