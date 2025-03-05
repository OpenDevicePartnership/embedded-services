use embassy_executor::{Executor, Spawner};
use embassy_sync::once_lock::OnceLock;
use log::*;
use static_cell::StaticCell;

use embedded_cfu_protocol::protocol_definitions::{ComponentId, FwUpdateOfferCommand, FwVersion, MAX_CMPT_COUNT};
use embedded_services::cfu;
use embedded_services::cfu::component::{CfuComponentDefault, CfuWrapperDefault};
use heapless::Vec;

#[embassy_executor::task]
async fn device_task0(wrapper: &'static CfuWrapperDefault<CfuComponentDefault>) {
    loop {
        if let Err(e) = wrapper.component.borrow_mut().process_request().await {
            error!("Error processing request: {:?}", e);
        }
    }
}

#[embassy_executor::task]
async fn device_task1(wrapper: &'static CfuWrapperDefault<CfuComponentDefault>) {
    loop {
        if let Err(e) = wrapper.component.borrow_mut().process_request().await {
            error!("Error processing request: {:?}", e);
        }
    }
}

#[embassy_executor::task]
async fn run(spawner: Spawner) {
    embedded_services::init().await;

    info!("Creating device 0");
    static DEVICE0: OnceLock<CfuWrapperDefault<CfuComponentDefault>> = OnceLock::new();
    let mut subs: Vec<ComponentId, { MAX_CMPT_COUNT - 1 }> = Vec::new();
    let _ = subs.push(2);
    static INNER0: OnceLock<CfuComponentDefault> = OnceLock::new();
    let inner = INNER0.get_or_init(|| CfuComponentDefault::new(1, true, Some(subs.clone())));
    cfu::register_device(inner).await.unwrap();
    let device0 = DEVICE0.get_or_init(|| CfuWrapperDefault::new(CfuComponentDefault::new(1, true, Some(subs))));
    //cfu::register_device(&device0.component).await.unwrap(); //doesn't build
    spawner.must_spawn(device_task0(device0));

    info!("Creating device 1");
    static DEVICE1: OnceLock<CfuWrapperDefault<CfuComponentDefault>> = OnceLock::new();
    static INNER1: OnceLock<CfuComponentDefault> = OnceLock::new();
    let inner = INNER1.get_or_init(|| CfuComponentDefault::new(2, false, None));
    cfu::register_device(inner).await.unwrap();
    let device1: &CfuWrapperDefault<CfuComponentDefault> =
        DEVICE1.get_or_init(|| CfuWrapperDefault::new(CfuComponentDefault::new(2, false, None)));
    // THIS DOESN'T BUILD because RefCell<T> doesn't implement the same traits as &T apparently
    //cfu::register_device(&device1.component).await.unwrap();
    spawner.must_spawn(device_task1(device1));

    let dummy_offer0 = FwUpdateOfferCommand::new(
        0,
        1,
        FwVersion {
            major: 1,
            minor: 23,
            variant: 45,
        },
    );
    let dummy_offer1 = FwUpdateOfferCommand::new(
        0,
        2,
        FwVersion {
            major: 1,
            minor: 23,
            variant: 45,
        },
    );

    match cfu::send_request(1, cfu::component::RequestData::GiveOffer(dummy_offer0)).await {
        Ok(resp) => {
            info!("got okay response to device0 update {:?}", resp);
        }
        Err(e) => {
            error!("offer failed with error {:?}", e);
        }
    }
    match cfu::send_request(2, cfu::component::RequestData::GiveOffer(dummy_offer1)).await {
        Ok(resp) => {
            info!("got okay response to device1 update {:?}", resp);
        }
        Err(e) => {
            error!("device1 offer failed with error {:?}", e);
        }
    }
}

fn main() {
    env_logger::builder().filter_level(log::LevelFilter::Info).init();

    static EXECUTOR: StaticCell<Executor> = StaticCell::new();
    let executor = EXECUTOR.init(Executor::new());
    executor.run(|spawner| {
        spawner.must_spawn(cfu_service::task());
        spawner.must_spawn(run(spawner));
    });
}
