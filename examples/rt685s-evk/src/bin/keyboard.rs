#![no_std]
#![no_main]

use defmt::info;
use embassy_executor::Spawner;
use embassy_imxrt;

mod backlight {
    use defmt::info;
    use embedded_services::activity::{Class, State, SubscriberHandle};

    pub struct Context {
        pub activity_handle: SubscriberHandle,
    }

    pub async fn init() -> Context {
        Context {
            activity_handle: embedded_services::activity::subscribe().await.ok().unwrap(),
        }
    }

    pub async fn turn_on() {
        info!("Backlight on!");
        embassy_time::Timer::after_millis(500).await;
    }

    pub async fn turn_off() {
        info!("Backlight off!");
        embassy_time::Timer::after_millis(500).await;
    }

    #[embassy_executor::task]
    pub async fn process_activity_event(mut handle: SubscriberHandle) {
        loop {
            let activity = embedded_services::activity::wait(&mut handle).await;

            match activity.class {
                Class::Keyboard => match activity.state {
                    State::Active => turn_on().await,
                    _ => turn_off().await,
                },
                _ => (), // don't care otherwise
            }
        }
    }
}

mod keyboard {
    use defmt::info;
    use embedded_services::activity::{Class, PublisherHandle, State};

    pub struct Context {
        publisher_handle: PublisherHandle,
    }

    pub async fn init() -> Context {
        Context {
            publisher_handle: embedded_services::activity::register_publisher(Class::Keyboard)
                .await
                .unwrap(),
        }
    }

    #[embassy_executor::task]
    pub async fn keyscan_loop(context: Context) {
        loop {
            use embassy_time::Timer;

            info!("Setting Keyboard Activity to ACTIVE");
            embedded_services::activity::publish(&context.publisher_handle, State::Active).await;
            Timer::after_secs(2).await;

            info!("Setting Keyboard Activity to INACTIVE");
            embedded_services::activity::publish(&context.publisher_handle, State::Inactive).await;
            Timer::after_secs(2).await;
        }
    }
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let _p = embassy_imxrt::init(Default::default());

    embedded_services::init();

    info!("Platform initialization complete ...");

    let kb = keyboard::init().await;

    spawner.spawn(keyboard::keyscan_loop(kb)).unwrap();

    let bl = backlight::init().await;

    spawner
        .spawn(backlight::process_activity_event(bl.activity_handle))
        .unwrap();

    embedded_services_examples::delay(10_000);
}
