#![no_std]
#![no_main]

extern crate rt685s_evk_example;

use defmt::info;
use embassy_executor::Spawner;
use embassy_imxrt::gpio::{self, Input, Inverter, Pull};
use embassy_sync::blocking_mutex::raw::ThreadModeRawMutex;
use embassy_sync::signal::Signal;
use embassy_time::Duration;
use embedded_services::comms::{self, EndpointID, Internal};
use power_button_service::button::{Button, ButtonConfig};
use power_button_service::button_interpreter::{Message, check_button_press};
use power_button_service::debounce::{ActiveState, Debouncer};
use static_cell::StaticCell;
use {defmt_rtt as _, panic_probe as _};

mod sender {
    use super::*;

    pub struct Sender {
        pub tp: comms::Endpoint,
        sn: Signal<ThreadModeRawMutex, Message>,
    }

    impl Sender {
        pub fn new() -> Self {
            Self {
                tp: comms::Endpoint::uninit(EndpointID::Internal(Internal::Power)),
                sn: Signal::new(),
            }
        }

        pub async fn send(&self, message: Message) {
            self.tp
                .send(EndpointID::Internal(Internal::Power), &message)
                .await
                .unwrap();
        }
    }

    impl comms::MailboxDelegate for Sender {
        fn receive(&self, message: &comms::Message) -> Result<(), comms::MailboxDelegateError> {
            let sig = message
                .data
                .get::<Message>()
                .ok_or(comms::MailboxDelegateError::MessageNotFound)?;

            self.sn.signal(*sig);

            Ok(())
        }
    }
}

mod receiver {
    use super::*;

    pub struct Receiver {
        pub tp: comms::Endpoint,
        pub sn: Signal<ThreadModeRawMutex, Message>,
    }

    impl Receiver {
        pub fn new() -> Self {
            Self {
                tp: comms::Endpoint::uninit(EndpointID::Internal(Internal::Power)),
                sn: Signal::new(),
            }
        }
    }

    impl comms::MailboxDelegate for Receiver {
        fn receive(&self, message: &comms::Message) -> Result<(), comms::MailboxDelegateError> {
            let sig = message
                .data
                .get::<Message>()
                .ok_or(comms::MailboxDelegateError::MessageNotFound)?;

            self.sn.signal(*sig);

            Ok(())
        }
    }
}

#[embassy_executor::task(pool_size = 4)]
async fn button_task(gpio: Input<'static>, config: ButtonConfig) {
    static SENDER: StaticCell<sender::Sender> = StaticCell::new();
    let sender = SENDER.init(sender::Sender::new());
    let mut button = Button::new(gpio, config);

    loop {
        match check_button_press(&mut button).await {
            Some(Message::ShortPress) => {
                info!("Short press");
                sender.send(Message::ShortPress).await;
            }
            Some(Message::LongPress) => {
                info!("Long press");
                sender.send(Message::LongPress).await;
            }
            Some(Message::PressAndHold) => {
                info!("Press and hold");
                sender.send(Message::PressAndHold).await;
            }
            None => {}
        }
    }
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    embedded_services::init().await;

    let p = embassy_imxrt::init(Default::default());

    // Create a power button instance
    let button_a = Input::new(p.PIO1_1, Pull::Up, Inverter::Disabled);
    // Create a debouncer instance
    let debouncer = Debouncer::new(3, Duration::from_millis(10), ActiveState::ActiveLow);
    // Create a custom button configuration instance
    let config_a = ButtonConfig::new(debouncer, Duration::from_millis(1000), Duration::from_millis(2000));

    // Create a second button instance
    let button_b = Input::new(p.PIO0_10, Pull::Up, Inverter::Disabled);
    // Create a default button configuration instance
    let config_b = ButtonConfig::default();

    // Spawn the button tasks
    spawner.must_spawn(button_task(button_a, config_a));
    spawner.must_spawn(button_task(button_b, config_b));

    static RECEIVER: StaticCell<receiver::Receiver> = StaticCell::new();
    let receiver = RECEIVER.init(receiver::Receiver::new());

    comms::register_endpoint(receiver, &receiver.tp).await.unwrap();

    // Create an LED instance
    let mut led_r = gpio::Output::new(
        p.PIO0_31,
        gpio::Level::Low,
        gpio::DriveMode::PushPull,
        gpio::DriveStrength::Normal,
        gpio::SlewRate::Standard,
    );

    // Create an LED instance
    let mut led_g = gpio::Output::new(
        p.PIO0_14,
        gpio::Level::Low,
        gpio::DriveMode::PushPull,
        gpio::DriveStrength::Normal,
        gpio::SlewRate::Standard,
    );

    // Create an LED instance
    let mut led_b = gpio::Output::new(
        p.PIO0_26,
        gpio::Level::Low,
        gpio::DriveMode::PushPull,
        gpio::DriveStrength::Normal,
        gpio::SlewRate::Standard,
    );

    loop {
        let msg = receiver.sn.wait().await;

        // Toggle the LEDs based on the button press duration
        match msg {
            Message::ShortPress => {
                led_g.toggle();
            }
            Message::LongPress => {
                led_b.toggle();
            }
            Message::PressAndHold => {
                led_r.toggle();
            }
        }
    }
}
