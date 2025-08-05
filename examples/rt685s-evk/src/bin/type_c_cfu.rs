#![no_std]
#![no_main]

use ::tps6699x::{ADDR1, TPS66994_NUM_PORTS};
use embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice;
use embassy_executor::Spawner;
use embassy_imxrt::gpio::{Input, Inverter, Pull};
use embassy_imxrt::i2c::Async;
use embassy_imxrt::i2c::master::{Config, I2cMaster};
use embassy_imxrt::{bind_interrupts, peripherals};
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::mutex::Mutex;
use embassy_time::Timer;
use embassy_time::{self as _, Delay};
use embedded_cfu_protocol::protocol_definitions::*;
use embedded_cfu_protocol::protocol_definitions::{FwUpdateOffer, FwUpdateOfferResponse, FwVersion};
use embedded_services::cfu::component::InternalResponseData;
use embedded_services::cfu::component::RequestData;
use embedded_services::power::policy::DeviceId as PowerId;
use embedded_services::type_c::{self, ControllerId};
use embedded_services::{GlobalRawMutex, cfu};
use embedded_services::{error, info};
use embedded_usb_pd::GlobalPortId;
use static_cell::StaticCell;
use tps6699x::asynchronous::embassy as tps6699x;
use type_c_service::driver::tps6699x::{self as tps6699x_drv};
use type_c_service::wrapper::backing::{BackingDefault, BackingDefaultStorage};

extern crate rt685s_evk_example;

bind_interrupts!(struct Irqs {
    FLEXCOMM2 => embassy_imxrt::i2c::InterruptHandler<peripherals::FLEXCOMM2>;
});

struct Validator;

impl type_c_service::wrapper::FwOfferValidator for Validator {
    fn validate(&self, _current: FwVersion, _offer: &FwUpdateOffer) -> FwUpdateOfferResponse {
        // For this example, we always accept the offer
        FwUpdateOfferResponse::new_accept(HostToken::Driver)
    }
}

type BusMaster<'a> = I2cMaster<'a, Async>;
type BusDevice<'a> = I2cDevice<'a, NoopRawMutex, BusMaster<'a>>;
type Wrapper<'a> =
    tps6699x_drv::Tps66994Wrapper<'a, NoopRawMutex, BusDevice<'a>, BackingDefault<'a, TPS66994_NUM_PORTS>, Validator>;
type Controller<'a> = tps6699x::controller::Controller<NoopRawMutex, BusDevice<'a>>;
type Interrupt<'a> = tps6699x::Interrupt<'a, NoopRawMutex, BusDevice<'a>>;

const CONTROLLER0_ID: ControllerId = ControllerId(0);
const CONTROLLER0_CFU_ID: ComponentId = 0x12;
const PORT0_ID: GlobalPortId = GlobalPortId(0);
const PORT1_ID: GlobalPortId = GlobalPortId(1);
const PORT0_PWR_ID: PowerId = PowerId(0);
const PORT1_PWR_ID: PowerId = PowerId(1);

#[embassy_executor::task]
async fn pd_controller_task(controller: &'static Wrapper<'static>) {
    loop {
        if let Err(e) = controller.process_next_event().await {
            error!("Error processing controller event: {:?}", e);
        }
    }
}

#[embassy_executor::task]
async fn interrupt_task(mut int_in: Input<'static>, mut interrupt: Interrupt<'static>) {
    tps6699x::task::interrupt_task(&mut int_in, &mut [&mut interrupt]).await;
}

#[embassy_executor::task]
async fn fw_update_task() {
    Timer::after_millis(1000).await;
    let context = cfu::ContextToken::create().unwrap();
    let device = context.get_device(CONTROLLER0_CFU_ID).await.unwrap();

    info!("Getting FW version");
    let response = device
        .execute_device_request(RequestData::FwVersionRequest)
        .await
        .unwrap();
    let prev_version = match response {
        InternalResponseData::FwVersionResponse(GetFwVersionResponse { component_info, .. }) => {
            Into::<u32>::into(component_info[0].fw_version)
        }
        _ => panic!("Unexpected response"),
    };
    info!("Got version: {:#x}", prev_version);

    info!("Giving offer");
    let offer = device
        .execute_device_request(RequestData::GiveOffer(FwUpdateOffer::new(
            HostToken::Driver,
            CONTROLLER0_CFU_ID,
            FwVersion::new(0x211),
            0,
            0,
        )))
        .await
        .unwrap();
    info!("Got response: {:?}", offer);

    let fw = &[]; //include_bytes!("../../fw.bin");
    let num_chunks = fw.len() / DEFAULT_DATA_LENGTH;

    for (i, chunk) in fw.chunks(DEFAULT_DATA_LENGTH).enumerate() {
        let header = FwUpdateContentHeader {
            data_length: chunk.len() as u8,
            sequence_num: i as u16,
            firmware_address: (i * DEFAULT_DATA_LENGTH) as u32,
            flags: if i == 0 {
                FW_UPDATE_FLAG_FIRST_BLOCK
            } else if i == num_chunks - 1 {
                FW_UPDATE_FLAG_LAST_BLOCK
            } else {
                0
            },
        };

        let mut chunk_data = [0u8; DEFAULT_DATA_LENGTH];
        chunk_data[..chunk.len()].copy_from_slice(chunk);
        let request = FwUpdateContentCommand {
            header,
            data: chunk_data,
        };

        info!("Sending chunk {} of {}", i, fw.len());
        let response = device
            .execute_device_request(RequestData::GiveContent(request))
            .await
            .unwrap();
        info!("Got response: {:?}", response);
    }

    Timer::after_millis(2000).await;
    info!("Getting FW version");
    let response = device
        .execute_device_request(RequestData::FwVersionRequest)
        .await
        .unwrap();
    let version = match response {
        InternalResponseData::FwVersionResponse(GetFwVersionResponse { component_info, .. }) => {
            Into::<u32>::into(component_info[0].fw_version)
        }
        _ => panic!("Unexpected response"),
    };
    info!("Got previous version: {:#x}", prev_version);
    info!("Got version: {:#x}", version);
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_imxrt::init(Default::default());

    info!("Embedded service init");
    embedded_services::init().await;

    type_c::controller::init();

    info!("Spawining power policy task");
    spawner.must_spawn(power_policy_service::task(Default::default()));

    info!("Spawining type-c service task");
    spawner.must_spawn(type_c_service::task());

    let int_in = Input::new(p.PIO1_7, Pull::Up, Inverter::Disabled);
    static BUS: StaticCell<Mutex<NoopRawMutex, BusMaster<'static>>> = StaticCell::new();
    let bus = BUS.init(Mutex::new(
        I2cMaster::new_async(p.FLEXCOMM2, p.PIO0_18, p.PIO0_17, Irqs, Config::default(), p.DMA0_CH5).unwrap(),
    ));

    let device = I2cDevice::new(bus);

    static CONTROLLER: StaticCell<Controller<'static>> = StaticCell::new();
    let controller = CONTROLLER.init(Controller::new_tps66994(device, ADDR1).unwrap());
    let (mut tps6699x, interrupt) = controller.make_parts();

    info!("Resetting PD controller");
    let mut delay = Delay;
    tps6699x.reset(&mut delay).await.unwrap();

    info!("Spawining interrupt task");
    spawner.must_spawn(interrupt_task(int_in, interrupt));

    // These aren't enabled by default
    tps6699x
        .modify_interrupt_mask_all(|mask| {
            mask.set_am_entered(true);
            mask.set_dp_sid_status_updated(true);
            mask.set_intel_vid_status_updated(true);
            mask.set_usb_status_updated(true);
            mask.set_power_path_switch_changed(true);
            *mask
        })
        .await
        .unwrap();

    static PD_PORTS: [GlobalPortId; 2] = [PORT0_ID, PORT1_ID];
    static BACKING_STORAGE: StaticCell<BackingDefaultStorage<TPS66994_NUM_PORTS, GlobalRawMutex>> = StaticCell::new();
    let backing_storage = BACKING_STORAGE.init(BackingDefaultStorage::new());
    let backing = backing_storage.get_backing().expect("Failed to create backing storage");

    info!("Spawining PD controller task");
    static PD_CONTROLLER: StaticCell<Wrapper> = StaticCell::new();
    let pd_controller = PD_CONTROLLER.init(
        tps6699x_drv::tps66994(
            tps6699x,
            CONTROLLER0_ID,
            &PD_PORTS,
            [PORT0_PWR_ID, PORT1_PWR_ID],
            CONTROLLER0_CFU_ID,
            backing,
            Default::default(),
            Validator,
        )
        .unwrap(),
    );

    pd_controller.register().await.unwrap();
    spawner.must_spawn(pd_controller_task(pd_controller));

    spawner.must_spawn(fw_update_task());
}
