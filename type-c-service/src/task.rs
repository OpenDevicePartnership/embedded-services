use core::future::Future;

use embassy_sync::once_lock::OnceLock;
use embedded_services::{error, info};

use crate::service::config::Config;
use crate::service::Service;

/// Task to run the Type-C service, takes a closure to customize the event loop
pub async fn task_closure<'a, Fut: Future<Output = ()>, F: Fn(&'a Service) -> Fut>(config: Config, f: F) {
    info!("Starting type-c task");

    let service = Service::create(config);
    let service = match service {
        Some(service) => service,
        None => {
            error!("Type-C service already initialized");
            return;
        }
    };

    static SERVICE: OnceLock<Service> = OnceLock::new();
    let service = SERVICE.get_or_init(|| service);

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
