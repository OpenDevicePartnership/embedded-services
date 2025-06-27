use embassy_executor::{Executor, Spawner};
use embassy_sync::once_lock::OnceLock;
use embedded_cfu_protocol::CfuWriterDefault;
use log::*;
use static_cell::StaticCell;

use embedded_cfu_protocol::protocol_definitions::{
    ComponentId, FwUpdateOffer, FwVersion, HostToken, MAX_SUBCMPT_COUNT,
};
use embedded_services::cfu;
use embedded_services::cfu::component::CfuComponentDefault;

use crate::cfu::component::RequestData;

#[embassy_executor::task]
async fn device_task0(component: &'static CfuComponentDefault<CfuWriterDefault>) {
    loop {
        if let Err(e) = component.process_request().await {
            error!("Error processing request: {e:?}");
        }
    }
}

#[embassy_executor::task]
async fn device_task1(component: &'static CfuComponentDefault<CfuWriterDefault>) {
    loop {
        if let Err(e) = component.process_request().await {
            error!("Error processing request: {e:?}");
        }
    }
}

#[embassy_executor::task]
async fn run(spawner: Spawner) {
    embedded_services::init().await;

    info!("Creating device 0");
    static DEVICE0: OnceLock<CfuComponentDefault<CfuWriterDefault>> = OnceLock::new();
    let mut subs: [Option<ComponentId>; MAX_SUBCMPT_COUNT] = [None; MAX_SUBCMPT_COUNT];
    subs[0] = Some(2);
    let device0 = DEVICE0.get_or_init(|| CfuComponentDefault::new(1, true, subs, CfuWriterDefault::new()));
    cfu::register_device(device0).await.unwrap();
    spawner.must_spawn(device_task0(device0));

    info!("Creating device 1");
    static DEVICE1: OnceLock<CfuComponentDefault<CfuWriterDefault>> = OnceLock::new();
    let device1 =
        DEVICE1.get_or_init(|| CfuComponentDefault::new(2, false, [None; MAX_SUBCMPT_COUNT], CfuWriterDefault::new()));
    cfu::register_device(device1).await.unwrap();
    spawner.must_spawn(device_task1(device1));

    let dummy_offer0 = FwUpdateOffer::new(
        HostToken::Driver,
        1,
        FwVersion {
            major: 1,
            minor: 23,
            variant: 45,
        },
        0,
        0,
    );
    let dummy_offer1 = FwUpdateOffer::new(
        HostToken::Driver,
        2,
        FwVersion {
            major: 1,
            minor: 23,
            variant: 45,
        },
        0,
        0,
    );

    match cfu::route_request(1, RequestData::GiveOffer(dummy_offer0)).await {
        Ok(resp) => {
            info!("got okay response to device0 update {resp:?}");
        }
        Err(e) => {
            error!("offer failed with error {e:?}");
        }
    }
    match cfu::route_request(2, RequestData::GiveOffer(dummy_offer1)).await {
        Ok(resp) => {
            info!("got okay response to device1 update {resp:?}");
        }
        Err(e) => {
            error!("device1 offer failed with error {e:?}");
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
