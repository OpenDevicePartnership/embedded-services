use embassy_executor::{Executor, Spawner};
use embassy_sync::once_lock::OnceLock;
use embedded_cfu_protocol::writer::CfuWriterNop;
use log::*;
use static_cell::StaticCell;

use embedded_cfu_protocol::protocol_definitions::{
    ComponentId, FwUpdateOffer, FwVersion, HostToken, MAX_SUBCMPT_COUNT,
};

use cfu_service::{
    CfuClient,
    component::{CfuComponentDefault, RequestData},
};

#[embassy_executor::task]
async fn device_task0(component: &'static CfuComponentDefault<CfuWriterNop>, cfu_client: &'static CfuClient) {
    loop {
        if let Err(e) = component.process_request(cfu_client).await {
            error!("Error processing request: {e:?}");
        }
    }
}

#[embassy_executor::task]
async fn device_task1(component: &'static CfuComponentDefault<CfuWriterNop>, cfu_client: &'static CfuClient) {
    loop {
        if let Err(e) = component.process_request(cfu_client).await {
            error!("Error processing request: {e:?}");
        }
    }
}

#[embassy_executor::task]
async fn run(spawner: Spawner) {
    embedded_services::init().await;

    static CFU_CLIENT: OnceLock<CfuClient> = OnceLock::new();
    let cfu_client = CfuClient::new(&CFU_CLIENT).await;

    spawner.must_spawn(cfu_service_task(cfu_client));

    info!("Creating device 0");
    static DEVICE0: OnceLock<CfuComponentDefault<CfuWriterNop>> = OnceLock::new();
    let mut subs: [Option<ComponentId>; MAX_SUBCMPT_COUNT] = [None; MAX_SUBCMPT_COUNT];
    subs[0] = Some(2);
    let device0 = DEVICE0.get_or_init(|| CfuComponentDefault::new(1, true, subs, CfuWriterNop {}));
    cfu_client.register_device(device0).unwrap();
    spawner.must_spawn(device_task0(device0, cfu_client));

    info!("Creating device 1");
    static DEVICE1: OnceLock<CfuComponentDefault<CfuWriterNop>> = OnceLock::new();
    let device1 =
        DEVICE1.get_or_init(|| CfuComponentDefault::new(2, false, [None; MAX_SUBCMPT_COUNT], CfuWriterNop {}));
    cfu_client.register_device(device1).unwrap();
    spawner.must_spawn(device_task1(device1, cfu_client));

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

    match cfu_client.route_request(1, RequestData::GiveOffer(dummy_offer0)).await {
        Ok(resp) => {
            info!("got okay response to device0 update {resp:?}");
        }
        Err(e) => {
            error!("offer failed with error {e:?}");
        }
    }
    match cfu_client.route_request(2, RequestData::GiveOffer(dummy_offer1)).await {
        Ok(resp) => {
            info!("got okay response to device1 update {resp:?}");
        }
        Err(e) => {
            error!("device1 offer failed with error {e:?}");
        }
    }
}

#[embassy_executor::task]
async fn cfu_service_task(cfu_client: &'static CfuClient) -> ! {
    cfu_service::task::task(cfu_client).await;
    unreachable!()
}

fn main() {
    env_logger::builder().filter_level(log::LevelFilter::Info).init();

    static EXECUTOR: StaticCell<Executor> = StaticCell::new();
    let executor = EXECUTOR.init(Executor::new());
    executor.run(|spawner| {
        spawner.must_spawn(run(spawner));
    });
}
