#![no_std]
#![no_main]

extern crate rt685s_evk_example;

use defmt::info;
use embassy_executor::Spawner;

mod simple_example {
    use embassy_sync::blocking_mutex::raw::NoopRawMutex;
    use embassy_sync::signal::Signal;
    use embedded_services::comms;
    use static_cell::StaticCell;

    use super::*;

    #[derive(Copy, Clone, Debug, defmt::Format)]
    enum Key {
        Sender,
        Receiver,
    }

    impl From<Key> for comms::OemKey {
        fn from(value: Key) -> Self {
            match value {
                Key::Sender => 0,
                Key::Receiver => 1,
            }
        }
    }

    impl From<Key> for comms::Internal {
        fn from(value: Key) -> Self {
            comms::Internal::Oem(value.into())
        }
    }

    impl From<Key> for comms::EndpointID {
        fn from(value: Key) -> Self {
            comms::EndpointID::Internal(value.into())
        }
    }

    #[derive(Copy, Clone, Debug, defmt::Format)]
    enum Signals {
        Command,
        Request,
        Response,
        Notification,
    }

    struct Context {
        tp: comms::Endpoint,
        sn: Signal<NoopRawMutex, Signals>,
    }

    impl Context {
        fn new(key: Key) -> Self {
            Self {
                tp: comms::Endpoint::uninit(key.into()),
                sn: Signal::new(),
            }
        }
    }

    impl comms::MailboxDelegate for Context {
        fn receive(&self, message: &comms::Message) -> Result<(), comms::MailboxDelegateError> {
            let sig = message
                .data
                .get::<Signals>()
                .ok_or(comms::MailboxDelegateError::MessageNotFound)?;

            self.sn.signal(*sig);

            Ok(())
        }
    }

    #[embassy_executor::task]
    pub async fn sender() {
        static SENDER: StaticCell<Context> = StaticCell::new();
        let this = SENDER.init(Context::new(Key::Sender));

        // register sender transport node
        comms::register_endpoint(this, &this.tp).await.unwrap();

        // wait for a second
        embassy_time::Timer::after_secs(1).await;

        // send command to receiver
        this.tp.send(Key::Receiver.into(), &Signals::Command).await.unwrap();

        loop {
            let sig = this.sn.wait().await;

            match sig {
                Signals::Command => info!("Sender: Unexpected command received!"),
                Signals::Notification => {
                    info!("Sender: received notification!");
                    embassy_time::Timer::after_secs(2).await;

                    info!("Sender: requesting receiver!");
                    this.tp.send(Key::Receiver.into(), &Signals::Request).await.unwrap();
                }
                Signals::Request => info!("Sender: Unexpected request received!"),
                Signals::Response => {
                    info!("Sender: got response!");
                    embassy_time::Timer::after_secs(2).await;

                    info!("Sender: commanding receiver!");
                    this.tp.send(Key::Receiver.into(), &Signals::Command).await.unwrap();
                }
            }
        }
    }

    #[embassy_executor::task]
    pub async fn receiver() {
        static RECEIVER: StaticCell<Context> = StaticCell::new();
        let this = RECEIVER.init(Context::new(Key::Receiver));

        comms::register_endpoint(this, &this.tp).await.unwrap();

        loop {
            let sig = this.sn.wait().await;

            match sig {
                Signals::Command => {
                    info!("Receiver: Got command!");
                    embassy_time::Timer::after_secs(2).await;

                    info!("Receiver: Sending notification!");
                    this.tp.send(Key::Sender.into(), &Signals::Notification).await.unwrap();
                }
                Signals::Request => {
                    info!("Receiver: Got Request!");
                    embassy_time::Timer::after_secs(2).await;

                    info!("Receiver: Sending reply!");
                    this.tp.send(Key::Sender.into(), &Signals::Response).await.unwrap();
                }
                Signals::Notification => info!("Receiver: Unexpected notification!"),
                Signals::Response => info!("Receiver: unexpected response!"),
            }
        }
    }
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let _p = embassy_imxrt::init(Default::default());

    info!("Platform initialization complete ...");

    embedded_services::init().await;

    info!("Service initialization complete...");

    spawner.spawn(simple_example::receiver()).unwrap();
    spawner.spawn(simple_example::sender()).unwrap();

    info!("Subsystem initialization complete...");

    embassy_time::Timer::after_secs(1).await;

    loop {
        embassy_time::Timer::after_secs(10).await;
        info!("10s elapsed...");
    }
}
