#![no_std]
#![no_main]

use ::tps6699x::{ADDR1, TPS66994_NUM_PORTS};
use embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice;
use embassy_executor::Spawner;
use embassy_imxrt::gpio::{Input, Inverter, Pull};
use embassy_imxrt::i2c::Async;
use embassy_imxrt::i2c::master::{Config, I2cMaster};
use embassy_imxrt::{bind_interrupts, peripherals};
use embassy_sync::mutex::Mutex;
use embassy_sync::pubsub::PubSubChannel;
use embassy_time::{self as _, Delay};
use embedded_cfu_protocol::protocol_definitions::{FwUpdateOffer, FwUpdateOfferResponse, FwVersion, HostToken};
use embedded_services::power::policy::{CommsMessage, DeviceId as PowerId};
use embedded_services::type_c::{Cached, ControllerId};
use embedded_services::{GlobalRawMutex, IntrusiveList};
use embedded_services::{error, info};
use embedded_usb_pd::GlobalPortId;
use static_cell::StaticCell;
use tps6699x::asynchronous::embassy as tps6699x;
use type_c_service::driver::tps6699x::{self as tps6699x_drv};
use type_c_service::service::Service;
use type_c_service::wrapper::ControllerWrapper;
use type_c_service::wrapper::backing::{ReferencedStorage, Storage};

extern crate rt685s_evk_example;

const NUM_PD_CONTROLLERS: usize = 1;
const CONTROLLER0_ID: ControllerId = ControllerId(0);
const PORT0_ID: GlobalPortId = GlobalPortId(0);
const PORT1_ID: GlobalPortId = GlobalPortId(1);
const PORT0_PWR_ID: PowerId = PowerId(0);
const PORT1_PWR_ID: PowerId = PowerId(1);
const POLICY_CHANNEL_SIZE: usize = 1;

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
type BusDevice<'a> = I2cDevice<'a, GlobalRawMutex, BusMaster<'a>>;
type Tps6699xMutex<'a> = Mutex<GlobalRawMutex, tps6699x_drv::Tps6699x<'a, GlobalRawMutex, BusDevice<'a>>>;
type Wrapper<'a> = ControllerWrapper<'a, GlobalRawMutex, Tps6699xMutex<'a>, Validator, POLICY_CHANNEL_SIZE>;
type Controller<'a> = tps6699x::controller::Controller<GlobalRawMutex, BusDevice<'a>>;
type Interrupt<'a> = tps6699x::Interrupt<'a, GlobalRawMutex, BusDevice<'a>>;

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
async fn power_policy_service_task(policy: &'static power_policy_service::PowerPolicy<POLICY_CHANNEL_SIZE>) {
    power_policy_service::task::task(
        policy,
        None::<[&rt685s_evk_example::DummyPowerDevice<POLICY_CHANNEL_SIZE>; 0]>,
        None::<[&rt685s_evk_example::DummyCharger; 0]>,
    )
    .await
    .expect("Failed to start power policy service task");
}

#[embassy_executor::task]
async fn service_task(
    controller_context: &'static embedded_services::type_c::controller::Context,
    controllers: &'static IntrusiveList,
    wrappers: [&'static Wrapper<'static>; NUM_PD_CONTROLLERS],
    power_policy_context: &'static embedded_services::power::policy::policy::Context<POLICY_CHANNEL_SIZE>,
) {
    info!("Starting type-c task");

    // The service is the only receiver and we only use a DynImmediatePublisher, which doesn't take a publisher slot
    static POWER_POLICY_CHANNEL: StaticCell<PubSubChannel<GlobalRawMutex, CommsMessage, 4, 1, 0>> = StaticCell::new();

    let power_policy_channel = POWER_POLICY_CHANNEL.init(PubSubChannel::new());
    let power_policy_publisher = power_policy_channel.dyn_immediate_publisher();
    // Guaranteed to not panic since we initialized the channel above
    let power_policy_subscriber = power_policy_channel.dyn_subscriber().unwrap();

    let service = Service::create(
        type_c_service::service::config::Config::default(),
        controller_context,
        controllers,
        power_policy_publisher,
        power_policy_subscriber,
    );

    static SERVICE: StaticCell<Service> = StaticCell::new();
    let service = SERVICE.init(service);

    type_c_service::task::task(service, wrappers, power_policy_context).await;
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_imxrt::init(Default::default());

    info!("Embedded service init");
    embedded_services::init().await;

    static POWER_POLICY_SERVICE: StaticCell<power_policy_service::PowerPolicy<POLICY_CHANNEL_SIZE>> = StaticCell::new();
    let power_service = POWER_POLICY_SERVICE.init(power_policy_service::PowerPolicy::new(
        power_policy_service::Config::default(),
    ));

    info!("Spawining power policy task");
    spawner.must_spawn(power_policy_service_task(power_service));

    static CONTROLLER_LIST: StaticCell<IntrusiveList> = StaticCell::new();
    let controllers = CONTROLLER_LIST.init(IntrusiveList::new());
    static CONTEXT: StaticCell<embedded_services::type_c::controller::Context> = StaticCell::new();
    let controller_context = CONTEXT.init(embedded_services::type_c::controller::Context::new());

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

    static STORAGE: StaticCell<Storage<TPS66994_NUM_PORTS, GlobalRawMutex, POLICY_CHANNEL_SIZE>> = StaticCell::new();
    let storage = STORAGE.init(Storage::new(
        controller_context,
        CONTROLLER0_ID,
        0, // CFU component ID
        [(PORT0_ID, PORT0_PWR_ID), (PORT1_ID, PORT1_PWR_ID)],
        &power_service.context,
    ));

    static REFERENCED: StaticCell<ReferencedStorage<TPS66994_NUM_PORTS, GlobalRawMutex, POLICY_CHANNEL_SIZE>> =
        StaticCell::new();
    let referenced = REFERENCED.init(
        storage
            .create_referenced()
            .expect("Failed to create referenced storage"),
    );

    info!("Spawining PD controller task");
    static CONTROLLER_MUTEX: StaticCell<Tps6699xMutex<'_>> = StaticCell::new();
    let controller_mutex = CONTROLLER_MUTEX.init(Mutex::new(tps6699x_drv::tps66994(tps6699x, Default::default())));

    static WRAPPER: StaticCell<Wrapper> = StaticCell::new();
    let wrapper =
        WRAPPER.init(ControllerWrapper::try_new(controller_mutex, Default::default(), referenced, Validator).unwrap());

    info!("Spawining type-c service task");
    spawner.must_spawn(service_task(
        controller_context,
        controllers,
        [wrapper],
        &power_service.context,
    ));

    spawner.must_spawn(pd_controller_task(wrapper));

    // Sync our internal state with the hardware
    controller_context
        .sync_controller_state_external(CONTROLLER0_ID)
        .await
        .unwrap();

    embassy_time::Timer::after_secs(10).await;

    let status = controller_context
        .get_controller_status_external(CONTROLLER0_ID)
        .await
        .unwrap();

    info!("Controller status: {:?}", status);

    let status = controller_context
        .get_port_status_external(PORT0_ID, Cached(true))
        .await
        .unwrap();
    info!("Port status: {:?}", status);

    let status = controller_context
        .get_port_status_external(PORT1_ID, Cached(true))
        .await
        .unwrap();
    info!("Port status: {:?}", status);
}
