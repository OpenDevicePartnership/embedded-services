#![no_std]
#![no_main]

extern crate rt685s_evk_example;

use defmt::info;
use embassy_executor::Spawner;

use static_cell::StaticCell;

mod activity_example {
    use embassy_sync::blocking_mutex::raw::NoopRawMutex;
    use embassy_sync::signal::Signal;

    use super::*;

    pub mod backlight {

        use embedded_services::activity;

        use super::*;

        // conceivably these actions could be permuted or extended based on whatever the "real" subscriber needs to do
        enum Actions {
            TurnOnBacklight,
            TurnOffBacklight(bool),
        }

        pub struct BacklightContext {
            activity_subscription: activity::Subscriber,
            action_queue: Signal<NoopRawMutex, Actions>,
        }

        impl activity::ActivitySubscriber for BacklightContext {
            fn activity_update(&self, notif: &activity::Notification) {
                if matches!(notif.class, activity::Class::Keyboard) {
                    // IPC wake
                    //    Note: if depth is needed, use a channel instead
                    self.action_queue.signal(match notif.state {
                        activity::State::Active => Actions::TurnOnBacklight,
                        activity::State::Inactive => Actions::TurnOffBacklight(true),
                        activity::State::Disabled => Actions::TurnOffBacklight(false),
                    });
                }
            }
        }

        impl BacklightContext {
            pub fn new() -> Self {
                Self {
                    activity_subscription: activity::Subscriber::uninit(),
                    action_queue: Signal::new(),
                }
            }

            async fn init(&'static self) {
                activity::register_subscriber(self, &self.activity_subscription)
                    .await
                    .unwrap();
            }

            async fn turn_on(&self) {
                info!("Backlight enabled!");
                embassy_time::Timer::after_millis(500).await;
            }

            async fn turn_off_immediate(&self) {
                info!("Backlight off!");
                embassy_time::Timer::after_millis(200).await;
            }

            async fn fade_off(&self) {
                info!("Backlight fading off!");
                embassy_time::Timer::after_millis(2000).await;
            }

            async fn event_loop(&self) {
                loop {
                    let event = self.action_queue.wait().await;

                    match event {
                        Actions::TurnOnBacklight => self.turn_on().await,
                        Actions::TurnOffBacklight(fade) => {
                            if fade {
                                self.fade_off().await;
                            } else {
                                self.turn_off_immediate().await;
                            }
                        }
                    }
                }
            }
        }

        #[embassy_executor::task]
        pub async fn task() {
            static CONTEXT: StaticCell<BacklightContext> = StaticCell::new();
            let context = CONTEXT.init(BacklightContext::new());
            context.init().await;
            context.event_loop().await;
        }
    }

    pub mod publisher {
        use embedded_services::activity;

        use super::*;

        struct Keyboard {
            activity_publisher: activity::Publisher,
        }

        #[embassy_executor::task]
        pub async fn keyboard_task() {
            static KEYBOARD: StaticCell<Keyboard> = StaticCell::new();

            let k = embassy_futures::block_on(async {
                Keyboard {
                    activity_publisher: activity::register_publisher(activity::Class::Keyboard).await.unwrap(),
                }
            });
            let keyboard = KEYBOARD.init(k);

            let mut count = 0;
            loop {
                let some_times = [10, 100, 1000, 2000, 5000];

                embassy_time::Timer::after_millis(some_times[count % some_times.len()]).await;

                let state = match count % 3 {
                    1 => activity::State::Active,
                    2 => activity::State::Inactive,
                    _ => activity::State::Disabled,
                };

                keyboard.activity_publisher.publish(state).await;

                count += 1;
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

    // create an activity service subscriber
    spawner.spawn(activity_example::backlight::task()).unwrap();

    // create an activity service publisher
    spawner.spawn(activity_example::publisher::keyboard_task()).unwrap();

    info!("Subsystem initialization complete...");

    embassy_time::Timer::after_millis(1000).await;
}
