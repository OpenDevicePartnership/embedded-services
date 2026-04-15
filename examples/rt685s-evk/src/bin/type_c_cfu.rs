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
use embassy_sync::pubsub::{DynImmediatePublisher, DynSubscriber, PubSubChannel};
use embassy_time::Timer;
use embassy_time::{self as _, Delay};
use embedded_cfu_protocol::protocol_definitions::*;
use embedded_cfu_protocol::protocol_definitions::{FwUpdateOffer, FwUpdateOfferResponse, FwVersion};
use embedded_services::GlobalRawMutex;
use embedded_services::event::MapSender;
use embedded_services::{error, info};
use embedded_usb_pd::GlobalPortId;
use power_policy_interface::psu;
use power_policy_service::psu::ArrayEventReceivers;
use power_policy_service::service::registration::ArrayRegistration;
use static_cell::StaticCell;
use tps6699x::asynchronous::embassy as tps6699x;
use type_c_interface::port::ControllerId;
use type_c_interface::port::PortRegistration;
use type_c_interface::service::event::PortEvent as ServicePortEvent;
use type_c_service::driver::tps6699x::{self as tps6699x_drv, InterruptReceiver};
use type_c_service::service::{EventReceiver, Service};
use type_c_service::wrapper::ControllerWrapper;
use type_c_service::wrapper::backing::{IntermediateStorage, ReferencedStorage, Storage};
use type_c_service::wrapper::event_receiver::ArrayPortEventReceivers;
use type_c_service::wrapper::proxy::PowerProxyDevice;

extern crate rt685s_evk_example;

const CHANNEL_CAPACITY: usize = 4;

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
    DynamicSender<'a, power_policy_interface::psu::event::EventData>,
    Validator,
>;
type Controller<'a> = tps6699x::controller::Controller<GlobalRawMutex, BusDevice<'a>>;
type InterruptProcessor<'a> = tps6699x::interrupt::InterruptProcessor<'a, GlobalRawMutex, BusDevice<'a>>;

type PowerPolicySenderType = MapSender<
    power_policy_interface::service::event::Event<'static, DeviceType>,
    power_policy_interface::service::event::EventData,
    DynImmediatePublisher<'static, power_policy_interface::service::event::EventData>,
    fn(
        power_policy_interface::service::event::Event<'static, DeviceType>,
    ) -> power_policy_interface::service::event::EventData,
>;

type PowerPolicyReceiverType = DynSubscriber<'static, power_policy_interface::service::event::EventData>;

type PowerPolicyServiceType = Mutex<
    GlobalRawMutex,
    power_policy_service::service::Service<
        'static,
        ArrayRegistration<'static, DeviceType, 2, PowerPolicySenderType, 1>,
    >,
>;

type ServiceType = Service<'static>;

const NUM_PD_CONTROLLERS: usize = 1;
const CONTROLLER0_ID: ControllerId = ControllerId(0);
const CONTROLLER0_CFU_ID: ComponentId = 0x12;
const PORT0_ID: GlobalPortId = GlobalPortId(0);
const PORT1_ID: GlobalPortId = GlobalPortId(1);

#[embassy_executor::task]
async fn pd_controller_task(
    mut event_receiver: ArrayPortEventReceivers<
        'static,
        2,
        InterruptReceiver<'static, GlobalRawMutex, BusDevice<'static>>,
    >,
    wrapper: &'static Wrapper<'static>,
) {
    loop {
        let event = event_receiver.wait_event().await;

        let output = wrapper
            .process_event(
                &mut event_receiver.sink_ready_timeout,
                &mut event_receiver.cfu_event_receiver,
                event,
            )
            .await;
        if let Err(e) = output {
            error!("Error processing event: {:?}", e);
        }
        let output = output.unwrap();
        if let Err(e) = wrapper.finalize(&mut event_receiver.power_proxies, output).await {
            error!("Error finalizing output: {:?}", e);
        }
    }
}

#[embassy_executor::task]
async fn interrupt_task(mut int_in: Input<'static>, mut interrupt: InterruptProcessor<'static>) {
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
    psu_events: ArrayEventReceivers<'static, 2, DeviceType, DynamicReceiver<'static, psu::event::EventData>>,
    power_policy: &'static PowerPolicyServiceType,
) {
    power_policy_service::service::task::task(psu_events, power_policy).await;
}

#[embassy_executor::task]
async fn type_c_service_task(
    service: &'static Mutex<GlobalRawMutex, ServiceType>,
    event_receiver: EventReceiver<'static, PowerPolicyReceiverType>,
    wrappers: [&'static Wrapper<'static>; NUM_PD_CONTROLLERS],
    cfu_client: &'static CfuClient,
) {
    type_c_service::task::task(service, event_receiver, wrappers, cfu_client).await;
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
    let controller = CONTROLLER.init(Controller::new_tps66994(device, Default::default(), ADDR1).unwrap());
    let (mut tps6699x, interrupt_processor, interrupt_receiver) = controller.make_parts();

    info!("Resetting PD controller");
    let mut delay = Delay;
    tps6699x.reset(&mut delay).await.unwrap();

    info!("Spawining interrupt task");
    spawner.spawn(interrupt_task(int_in, interrupt_processor).expect("Failed to spawn interrupt task"));

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

    static CONTROLLER_CONTEXT: StaticCell<type_c_interface::service::context::Context> = StaticCell::new();
    let controller_context = CONTROLLER_CONTEXT.init(type_c_interface::service::context::Context::new());

    static PORT0_CHANNEL: Channel<GlobalRawMutex, ServicePortEvent, CHANNEL_CAPACITY> = Channel::new();
    static PORT1_CHANNEL: Channel<GlobalRawMutex, ServicePortEvent, CHANNEL_CAPACITY> = Channel::new();
    static STORAGE: StaticCell<Storage<TPS66994_NUM_PORTS, GlobalRawMutex>> = StaticCell::new();
    let storage = STORAGE.init(Storage::new(
        controller_context,
        CONTROLLER0_ID,
        CONTROLLER0_CFU_ID,
        [
            PortRegistration {
                id: PORT0_ID,
                sender: PORT0_CHANNEL.dyn_sender(),
                receiver: PORT0_CHANNEL.dyn_receiver(),
            },
            PortRegistration {
                id: PORT1_ID,
                sender: PORT1_CHANNEL.dyn_sender(),
                receiver: PORT1_CHANNEL.dyn_receiver(),
            },
        ],
    ));

    static POLICY_CHANNEL0: StaticCell<Channel<GlobalRawMutex, psu::event::EventData, 1>> = StaticCell::new();
    let policy_channel0 = POLICY_CHANNEL0.init(Channel::new());
    let policy_sender0 = policy_channel0.dyn_sender();
    let policy_receiver0 = policy_channel0.dyn_receiver();

    static POLICY_CHANNEL1: StaticCell<Channel<GlobalRawMutex, psu::event::EventData, 1>> = StaticCell::new();
    let policy_channel1 = POLICY_CHANNEL1.init(Channel::new());
    let policy_sender1 = policy_channel1.dyn_sender();
    let policy_receiver1 = policy_channel1.dyn_receiver();

    let (intermediate, power_event_receivers) = storage
        .try_create_intermediate([("Pd0", policy_sender0), ("Pd1", policy_sender1)])
        .expect("Failed to create intermediate storage");
    static INTERMEDIATE: StaticCell<
        IntermediateStorage<TPS66994_NUM_PORTS, GlobalRawMutex, DynamicSender<'static, psu::event::EventData>>,
    > = StaticCell::new();
    let intermediate = INTERMEDIATE.init(intermediate);

    static REFERENCED: StaticCell<
        ReferencedStorage<
            TPS66994_NUM_PORTS,
            GlobalRawMutex,
            DynamicSender<'_, power_policy_interface::psu::event::EventData>,
        >,
    > = StaticCell::new();
    let referenced = REFERENCED.init(
        intermediate
            .try_create_referenced()
            .expect("Failed to create referenced storage"),
    );

    info!("Spawining PD controller task");
    static CONTROLLER_MUTEX: StaticCell<Tps6699xMutex<'_>> = StaticCell::new();
    let controller_mutex = CONTROLLER_MUTEX.init(Mutex::new(tps6699x_drv::tps66994(
        tps6699x,
        Default::default(),
        Default::default(),
    )));

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

    // The service is the only receiver and we only use a DynImmediatePublisher, which doesn't take a publisher slot
    static POWER_POLICY_CHANNEL: StaticCell<
        PubSubChannel<GlobalRawMutex, power_policy_interface::service::event::EventData, 4, 1, 0>,
    > = StaticCell::new();

    let power_policy_channel = POWER_POLICY_CHANNEL.init(PubSubChannel::new());
    let power_policy_sender: PowerPolicySenderType =
        MapSender::new(power_policy_channel.dyn_immediate_publisher(), |e| e.into());
    // Guaranteed to not panic since we initialized the channel above
    let power_policy_subscriber = power_policy_channel.dyn_subscriber().unwrap();

    let power_policy_registration = ArrayRegistration {
        psus: [&wrapper.ports[0].proxy, &wrapper.ports[1].proxy],
        service_senders: [power_policy_sender],
    };

    static POWER_SERVICE: StaticCell<PowerPolicyServiceType> = StaticCell::new();
    let power_service = POWER_SERVICE.init(Mutex::new(power_policy_service::service::Service::new(
        power_policy_registration,
        power_service_context,
        power_policy_service::service::config::Config::default(),
    )));

    static TYPE_C_SERVICE: StaticCell<Mutex<GlobalRawMutex, ServiceType>> = StaticCell::new();
    let type_c_service = TYPE_C_SERVICE.init(Mutex::new(Service::create(Default::default(), controller_context)));

    // Spin up CFU service
    static CFU_CLIENT: OnceLock<CfuClient> = OnceLock::new();
    let cfu_client = CfuClient::new(&CFU_CLIENT).await;

    info!("Spawining type-c service task");
    spawner.spawn(
        type_c_service_task(
            type_c_service,
            EventReceiver::new(controller_context, power_policy_subscriber),
            [wrapper],
            cfu_client,
        )
        .expect("Failed to spawn type-c service task"),
    );

    info!("Spawining power policy task");
    spawner.spawn(
        power_policy_task(
            ArrayEventReceivers::new(
                [&wrapper.ports[0].proxy, &wrapper.ports[1].proxy],
                [policy_receiver0, policy_receiver1],
            ),
            power_service,
        )
        .expect("Failed to create power policy task"),
    );

    spawner.spawn(
        pd_controller_task(
            ArrayPortEventReceivers::new(
                InterruptReceiver::new(interrupt_receiver),
                power_event_receivers,
                &referenced.pd_controller,
                &storage.cfu_device,
            ),
            wrapper,
        )
        .expect("Failed to create pd controller task"),
    );

    spawner.spawn(fw_update_task().expect("Failed to create fw update task"));
}
