#![no_std]
#![no_main]

use ::tps6699x::{ADDR1, TPS66994_NUM_PORTS};
use cfu_service::CfuClient;
use cfu_service::component::{InternalResponseData, RequestData};
use embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice;
use embassy_executor::Spawner;
use embassy_imxrt::gpio::{Input, Inverter, Pull};
use embassy_imxrt::i2c::Async;
use embassy_imxrt::i2c::master::{Config, I2cMaster};
use embassy_imxrt::{bind_interrupts, peripherals};
use embassy_sync::channel::{Channel, DynamicReceiver, DynamicSender};
use embassy_sync::mutex::Mutex;
use embassy_sync::once_lock::OnceLock;
use embassy_sync::pubsub::PubSubChannel;
use embassy_time::Timer;
use embassy_time::{self as _, Delay};
use embedded_cfu_protocol::protocol_definitions::*;
use embedded_cfu_protocol::protocol_definitions::{FwUpdateOffer, FwUpdateOfferResponse, FwVersion};
use embedded_services::{GlobalRawMutex, IntrusiveList};
use embedded_services::{error, info};
use embedded_usb_pd::GlobalPortId;
use power_policy_service::psu;
use static_cell::StaticCell;
use tps6699x::asynchronous::embassy as tps6699x;
use type_c_service::driver::tps6699x::{self as tps6699x_drv};
use type_c_service::service::Service;
use type_c_service::type_c::ControllerId;
use type_c_service::wrapper::ControllerWrapper;
use type_c_service::wrapper::backing::{IntermediateStorage, ReferencedStorage, Storage};
use type_c_service::wrapper::proxy::PowerProxyDevice;

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

type DeviceType = Mutex<GlobalRawMutex, PowerProxyDevice<'static>>;

type BusMaster<'a> = I2cMaster<'a, Async>;
type BusDevice<'a> = I2cDevice<'a, GlobalRawMutex, BusMaster<'a>>;
type Tps6699xMutex<'a> = Mutex<GlobalRawMutex, tps6699x_drv::Tps6699x<'a, GlobalRawMutex, BusDevice<'a>>>;
type Wrapper<'a> = ControllerWrapper<
    'a,
    GlobalRawMutex,
    Tps6699xMutex<'a>,
    DynamicSender<'a, power_policy_service::psu::event::EventData>,
    Validator,
>;
type Controller<'a> = tps6699x::controller::Controller<GlobalRawMutex, BusDevice<'a>>;
type Interrupt<'a> = tps6699x::Interrupt<'a, GlobalRawMutex, BusDevice<'a>>;

const NUM_PD_CONTROLLERS: usize = 1;
const CONTROLLER0_ID: ControllerId = ControllerId(0);
const CONTROLLER0_CFU_ID: ComponentId = 0x12;
const PORT0_ID: GlobalPortId = GlobalPortId(0);
const PORT1_ID: GlobalPortId = GlobalPortId(1);

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
    let context = cfu_service::ClientContext::new();
    let device = context.get_device(CONTROLLER0_CFU_ID).unwrap();

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
    let num_chunks = fw.len() / DEFAULT_DATA_LENGTH + (fw.len() % DEFAULT_DATA_LENGTH != 0) as usize;

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

        info!("Sending chunk {} of {}", i + 1, num_chunks);
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

#[embassy_executor::task]
async fn power_policy_task(
    psu_events: psu::event::EventReceivers<'static, 2, DeviceType, DynamicReceiver<'static, psu::event::EventData>>,
    power_policy: &'static Mutex<GlobalRawMutex, power_policy_service::service::Service<'static, DeviceType>>,
) {
    power_policy_service::service::task::task(psu_events, power_policy).await;
}

#[embassy_executor::task]
async fn type_c_service_task(
    service: &'static Service<'static, DeviceType>,
    wrappers: [&'static Wrapper<'static>; NUM_PD_CONTROLLERS],
    power_policy_context: &'static power_policy_service::service::context::Context,
    cfu_client: &'static CfuClient,
) {
    info!("Starting type-c task");
    type_c_service::task::task(service, wrappers, power_policy_context, cfu_client).await;
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_imxrt::init(Default::default());

    info!("Embedded service init");
    embedded_services::init().await;

    let int_in = Input::new(p.PIO1_7, Pull::Up, Inverter::Disabled);
    static BUS: StaticCell<Mutex<GlobalRawMutex, BusMaster<'static>>> = StaticCell::new();
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
            mask.set_sink_ready(true);
            *mask
        })
        .await
        .unwrap();

    static CONTROLLER_CONTEXT: StaticCell<type_c_service::type_c::controller::Context> = StaticCell::new();
    let controller_context = CONTROLLER_CONTEXT.init(type_c_service::type_c::controller::Context::new());

    static CONTROLLER_LIST: StaticCell<IntrusiveList> = StaticCell::new();
    let controller_list = CONTROLLER_LIST.init(IntrusiveList::new());

    static STORAGE: StaticCell<Storage<TPS66994_NUM_PORTS, GlobalRawMutex>> = StaticCell::new();
    let storage = STORAGE.init(Storage::new(
        controller_context,
        CONTROLLER0_ID,
        CONTROLLER0_CFU_ID,
        [PORT0_ID, PORT1_ID],
    ));

    static POLICY_CHANNEL0: StaticCell<Channel<GlobalRawMutex, psu::event::EventData, 1>> = StaticCell::new();
    let policy_channel0 = POLICY_CHANNEL0.init(Channel::new());
    let policy_sender0 = policy_channel0.dyn_sender();
    let policy_receiver0 = policy_channel0.dyn_receiver();

    static POLICY_CHANNEL1: StaticCell<Channel<GlobalRawMutex, psu::event::EventData, 1>> = StaticCell::new();
    let policy_channel1 = POLICY_CHANNEL1.init(Channel::new());
    let policy_sender1 = policy_channel1.dyn_sender();
    let policy_receiver1 = policy_channel1.dyn_receiver();

    static INTERMEDIATE: StaticCell<
        IntermediateStorage<TPS66994_NUM_PORTS, GlobalRawMutex, DynamicSender<'static, psu::event::EventData>>,
    > = StaticCell::new();
    let intermediate = INTERMEDIATE.init(
        storage
            .try_create_intermediate([("Pd0", policy_sender0), ("Pd1", policy_sender1)])
            .expect("Failed to create intermediate storage"),
    );

    static REFERENCED: StaticCell<
        ReferencedStorage<
            TPS66994_NUM_PORTS,
            GlobalRawMutex,
            DynamicSender<'_, power_policy_service::psu::event::EventData>,
        >,
    > = StaticCell::new();
    let referenced = REFERENCED.init(
        intermediate
            .try_create_referenced()
            .expect("Failed to create referenced storage"),
    );

    info!("Spawining PD controller task");
    static CONTROLLER_MUTEX: StaticCell<Tps6699xMutex<'_>> = StaticCell::new();
    let controller_mutex = CONTROLLER_MUTEX.init(Mutex::new(tps6699x_drv::tps66994(tps6699x, Default::default())));

    static WRAPPER: StaticCell<Wrapper> = StaticCell::new();
    let wrapper = WRAPPER.init(ControllerWrapper::new(
        controller_mutex,
        Default::default(),
        referenced,
        Validator,
    ));

    // Create power policy service
    static POWER_SERVICE_CONTEXT: StaticCell<power_policy_service::service::context::Context> = StaticCell::new();
    let power_service_context = POWER_SERVICE_CONTEXT.init(power_policy_service::service::context::Context::new());

    static POWER_POLICY_PSU_REGISTRATION: StaticCell<[&DeviceType; 2]> = StaticCell::new();
    let psu_registration = POWER_POLICY_PSU_REGISTRATION.init([&wrapper.ports[0].proxy, &wrapper.ports[1].proxy]);

    static POWER_SERVICE: StaticCell<Mutex<GlobalRawMutex, power_policy_service::service::Service<DeviceType>>> =
        StaticCell::new();
    let power_service = POWER_SERVICE.init(Mutex::new(power_policy_service::service::Service::new(
        psu_registration,
        power_service_context,
        power_policy_service::service::config::Config::default(),
    )));

    // The service is the only receiver and we only use a DynImmediatePublisher, which doesn't take a publisher slot
    static POWER_POLICY_CHANNEL: StaticCell<
        PubSubChannel<GlobalRawMutex, power_policy_service::service::event::Event<'static, DeviceType>, 4, 1, 0>,
    > = StaticCell::new();

    let power_policy_channel = POWER_POLICY_CHANNEL.init(PubSubChannel::new());
    let power_policy_publisher = power_policy_channel.dyn_immediate_publisher();
    // Guaranteed to not panic since we initialized the channel above
    let power_policy_subscriber = power_policy_channel.dyn_subscriber().unwrap();

    static TYPE_C_SERVICE: StaticCell<Service<'static, DeviceType>> = StaticCell::new();
    let type_c_service = TYPE_C_SERVICE.init(Service::create(
        Default::default(),
        controller_context,
        controller_list,
        power_policy_publisher,
        power_policy_subscriber,
    ));

    // Spin up CFU service
    static CFU_CLIENT: OnceLock<CfuClient> = OnceLock::new();
    let cfu_client = CfuClient::new(&CFU_CLIENT).await;

    info!("Spawining type-c service task");
    spawner.must_spawn(type_c_service_task(
        type_c_service,
        [wrapper],
        power_service_context,
        cfu_client,
    ));

    info!("Spawining power policy task");
    spawner.must_spawn(power_policy_task(
        psu::event::EventReceivers::new(
            [&wrapper.ports[0].proxy, &wrapper.ports[1].proxy],
            [policy_receiver0, policy_receiver1],
        ),
        power_service,
    ));

    spawner.must_spawn(pd_controller_task(wrapper));

    spawner.must_spawn(fw_update_task());
}
