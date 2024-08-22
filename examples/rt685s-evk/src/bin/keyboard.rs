#![no_std]
#![no_main]

use defmt::info;
use embassy_executor::Spawner;
use embassy_sync::once_lock::OnceLock;
use {embassy_imxrt, embedded_services};
static SERVICES: OnceLock<embedded_services::Services<PlatformServices>> = OnceLock::new();

// todo: wrap in macro
pub struct PlatformServices {
    activity: embedded_services::DynamicService<embedded_services::activity::Manager, 2, 1>,
}
impl embedded_services::DynamicServiceBlock for PlatformServices {
    fn get(
        &self,
        service: embedded_services::DynamicServiceListing,
    ) -> Option<embedded_services::DynamicServiceInstance<'_>> {
        use embedded_services::*;
        match service {
            DynamicServiceListing::Activity => Some(DynamicServiceInstance::Activity(&self.activity)),
            _ => None,
        }
    }
}

async fn backlight_on() {
    info!("Backlight turned ON!");
    embassy_time::Timer::after_millis(500).await;
}

async fn backlight_off() {
    info!("Backlight turned OFF!");
    embassy_time::Timer::after_millis(500).await;
}

#[embassy_executor::task]
async fn backlight_activity_consumer() {
    use embedded_services::DynamicServiceBlock;

    let activity_service_enum = SERVICES
        .get()
        .await
        .dynamic
        .get(embedded_services::DynamicServiceListing::Activity)
        .unwrap();

    let activity_service = match activity_service_enum {
        embedded_services::DynamicServiceInstance::Activity(activity_service) => activity_service,
        _ => panic!(), // activity service not available on this platform!
    };

    let mut subscriber = activity_service.subscribe().unwrap();

    loop {
        use embedded_services::activity::{Class, State};
        let activity = subscriber.wait().await;

        match activity.class {
            Class::Keyboard => match activity.state {
                State::Active => backlight_on().await,
                _ => backlight_off().await,
            },
            _ => (), // don't care
        }
    }
}

async fn kick_screen_on() {
    info!("Telling OS to turn on screen!");
    embassy_time::Timer::after_millis(800).await;
}

#[embassy_executor::task]
async fn screen_activity_consumer() {
    use embedded_services::DynamicServiceBlock;

    let activity_service_enum = SERVICES
        .get()
        .await
        .dynamic
        .get(embedded_services::DynamicServiceListing::Activity)
        .unwrap();

    let activity_service = match activity_service_enum {
        embedded_services::DynamicServiceInstance::Activity(activity_service) => activity_service,
        _ => panic!(), // activity service not available on this platform!
    };

    let mut subscriber = activity_service.subscribe().unwrap();

    loop {
        use embedded_services::activity::{Class, State};
        let activity = subscriber.wait().await;

        match activity.class {
            Class::Keyboard => match activity.state {
                State::Active => kick_screen_on().await,
                _ => (), // nothing to do
            },
            _ => (), // don't care
        }
    }
}

#[embassy_executor::task]
async fn keyboard_activity_generator() {
    use embedded_services::DynamicServiceBlock;

    let activity_service_enum = SERVICES
        .get()
        .await
        .dynamic
        .get(embedded_services::DynamicServiceListing::Activity)
        .unwrap();

    let activity_service = match activity_service_enum {
        embedded_services::DynamicServiceInstance::Activity(activity_service) => activity_service,
        _ => panic!(), // activity service not available on this platform!
    };

    let publisher = activity_service.register_publisher().unwrap();

    loop {
        info!("Keyboard::Activity = ACTIVE!");
        publisher
            .publish(embedded_services::activity::Notification {
                state: embedded_services::activity::State::Active,
                class: embedded_services::activity::Class::Keyboard,
            })
            .await;
        embassy_time::Timer::after_secs(2).await;

        info!("Keyboard::Activity = INACTIVE!");
        publisher
            .publish(embedded_services::activity::Notification {
                state: embedded_services::activity::State::Inactive,
                class: embedded_services::activity::Class::Keyboard,
            })
            .await;
        embassy_time::Timer::after_secs(2).await;
    }
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let _p = embassy_imxrt::init(Default::default());

    info!("Platform initialization complete");

    SERVICES.get_or_init(|| {
        embedded_services::init(PlatformServices {
            activity: embedded_services::configure(embedded_services::activity::Config {}),
        })
    });

    info!("Service initialization complete");

    let _ = spawner.spawn(keyboard_activity_generator());
    let _ = spawner.spawn(backlight_activity_consumer());
    let _ = spawner.spawn(screen_activity_consumer());

    embedded_services_examples::delay(1_000);
    loop {
        embassy_time::Timer::after_secs(10).await;
    }
}
