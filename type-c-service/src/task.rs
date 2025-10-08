use core::future::Future;
use embassy_sync::pubsub::PubSubChannel;
use embedded_services::{GlobalRawMutex, error, info, power};
use static_cell::StaticCell;

use crate::service::config::Config;
use crate::service::{MAX_POWER_POLICY_EVENTS, Service};

/// Task to run the Type-C service, takes a closure to customize the event loop
pub async fn task_closure<'a, Fut: Future<Output = ()>, F: Fn(&'a Service) -> Fut>(config: Config, f: F) {
    info!("Starting type-c task");

    // The service is the only receiver and we only use a DynImmediatePublisher, which doesn't take a publisher slot
    static POWER_POLICY_CHANNEL: StaticCell<
        PubSubChannel<GlobalRawMutex, power::policy::CommsMessage, MAX_POWER_POLICY_EVENTS, 1, 0>,
    > = StaticCell::new();

    let power_policy_channel = POWER_POLICY_CHANNEL.init(PubSubChannel::new());
    let power_policy_publisher = power_policy_channel.dyn_immediate_publisher();
    // Guaranteed to not panic since we initialized the channel above
    let power_policy_subscriber = power_policy_channel.dyn_subscriber().unwrap();

    let service = Service::create(config, power_policy_publisher, power_policy_subscriber);
    let service = match service {
        Some(service) => service,
        None => {
            error!("Type-C service already initialized");
            return;
        }
    };

    static SERVICE: StaticCell<Service> = StaticCell::new();
    let service = SERVICE.init(service);

    if service.register_comms().await.is_err() {
        error!("Failed to register type-c service endpoint");
        return;
    }

    loop {
        f(service).await;
    }
}

#[embassy_executor::task]
pub async fn task(config: Config) {
    task_closure(config, |service: &Service| async {
        if let Err(e) = service.process_next_event().await {
            error!("Type-C service processing error: {:#?}", e);
        }
    })
    .await;
}
