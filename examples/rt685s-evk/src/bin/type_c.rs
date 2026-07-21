#![no_std]
#![no_main]
use ::tps6699x::ADDR1;
use ::tps6699x::asynchronous::embassy::interrupt::InterruptReceiver;
use embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice;
use embassy_executor::Spawner;
use embassy_imxrt::gpio::{Input, Inverter, Pull};
use embassy_imxrt::i2c::Async;
use embassy_imxrt::i2c::master::{Config, I2cMaster};
use embassy_imxrt::{bind_interrupts, peripherals};
use embassy_sync::channel::{DynamicReceiver, DynamicSender};
use embassy_sync::mutex::Mutex;
use embassy_sync::pubsub::{DynImmediatePublisher, DynSubscriber, PubSubChannel};
use embassy_time::{self as _, Delay};
use embedded_services::GlobalRawMutex;
use embedded_services::event::{MapSender, NoopSender};
use embedded_services::{error, info};
use embedded_usb_pd::LocalPortId;
use power_policy_interface::psu;
use power_policy_service::psu::PsuEventReceivers;
use power_policy_service::service::registration::ArrayRegistration;
use static_cell::StaticCell;
use tps6699x::odp::driver as tps6699x_drv;
use type_c_interface::port::event::PortEventBitfield;
use type_c_service::controller::Port;
use type_c_service::controller::event_receiver::{
    EventReceiver as PortEventReceiver, InterruptReceiver as _, PortEventSplitter,
};
use type_c_service::controller::macros::PortComponents;
use type_c_service::controller::state::SharedState;
use type_c_service::define_controller_port_static_cell_channel;
use type_c_service::service::Service;
use type_c_service::service::registration::PortData;

extern crate rt685s_evk_example;

bind_interrupts!(struct Irqs {
    FLEXCOMM2 => embassy_imxrt::i2c::InterruptHandler<peripherals::FLEXCOMM2>;
});

type PowerNotifier<'a> =
    power_policy_interface::psu::event::NonBlockingSenderNotifier<DynamicSender<'a, psu::event::EventData>>;
type SharedStateType = Mutex<GlobalRawMutex, SharedState>;
type PortType = Mutex<
    GlobalRawMutex,
    Port<
        'static,
        Tps6699xMutex<'static>,
        SharedStateType,
        DynamicSender<'static, type_c_interface::service::event::PortEventData>,
        PowerNotifier<'static>,
        DynamicSender<'static, type_c_service::controller::event::Loopback>,
    >,
>;
type ChargerType = power_policy_interface::charger::mock::ChargerType;

type BusMaster<'a> = I2cMaster<'a, Async>;
type BusDevice<'a> = I2cDevice<'a, GlobalRawMutex, BusMaster<'a>>;
type Tps6699xMutex<'a> = Mutex<GlobalRawMutex, tps6699x_drv::Tps6699x<'a, GlobalRawMutex, BusDevice<'a>>>;
type Controller<'a> = tps6699x::asynchronous::embassy::controller::Controller<GlobalRawMutex, BusDevice<'a>>;
type InterruptProcessor<'a> =
    tps6699x::asynchronous::embassy::interrupt::InterruptProcessor<'a, GlobalRawMutex, BusDevice<'a>>;

type PowerPolicySenderType = MapSender<
    power_policy_interface::service::event::Event<'static, PortType>,
    power_policy_interface::service::event::EventData,
    DynImmediatePublisher<'static, power_policy_interface::service::event::EventData>,
    fn(
        power_policy_interface::service::event::Event<'static, PortType>,
    ) -> power_policy_interface::service::event::EventData,
>;

type PowerPolicyReceiverType = DynSubscriber<'static, power_policy_interface::service::event::EventData>;
type PowerPolicyNotifierType =
    power_policy_interface::service::event::NonBlockingSenderNotifier<'static, PortType, PowerPolicySenderType>;
type PowerPolicyServiceType = Mutex<
    GlobalRawMutex,
    power_policy_service::service::Service<
        'static,
        ArrayRegistration<'static, PortType, 2, PowerPolicyNotifierType, 1, ChargerType, 0>,
    >,
>;

const PORT_COUNT: usize = 2;
type PortReceiverType = DynamicReceiver<'static, type_c_interface::service::event::PortEventData>;
type TypeCServiceEventReceiverType = type_c_service::service::event_receiver::ArrayEventReceiver<
    'static,
    PORT_COUNT,
    PortType,
    PortReceiverType,
    PowerPolicyReceiverType,
>;

type TypeCServiceSenderType = NoopSender;
type TypeCRegistrationType =
    type_c_service::service::registration::ArrayRegistration<'static, PortType, PORT_COUNT, TypeCServiceSenderType, 1>;
type TypeCServiceType = type_c_service::service::Service<'static, TypeCRegistrationType>;
type PortEventReceiverType = PortEventReceiver<
    'static,
    SharedStateType,
    DynamicReceiver<'static, PortEventBitfield>,
    DynamicReceiver<'static, type_c_service::controller::event::Loopback>,
>;

#[embassy_executor::task(pool_size = 2)]
async fn port_task(mut event_receiver: PortEventReceiverType, port: &'static PortType) {
    port.lock().await.sync_state().await.unwrap();

    loop {
        let event = event_receiver.wait_event().await;
        let output = port.lock().await.process_event(event).await;
        if let Err(e) = output {
            error!("Error processing event: {:?}", e);
        }
    }
}

#[embassy_executor::task]
async fn interrupt_task(mut int_in: Input<'static>, mut interrupt: InterruptProcessor<'static>) {
    tps6699x::asynchronous::embassy::task::interrupt_task(&mut int_in, &mut [&mut interrupt]).await;
}

#[embassy_executor::task]
async fn interrupt_splitter_task(
    mut interrupt_receiver: InterruptReceiver<'static, GlobalRawMutex, BusDevice<'static>>,
    mut interrupt_splitter: PortEventSplitter<2, DynamicSender<'static, PortEventBitfield>>,
) -> ! {
    loop {
        let interrupts = interrupt_receiver.wait_interrupt().await;
        interrupt_splitter.process_interrupts(interrupts).await;
    }
}

#[embassy_executor::task]
async fn power_policy_task(
    psu_events: PsuEventReceivers<'static, 2, PortType, DynamicReceiver<'static, psu::event::EventData>>,
    power_policy: &'static PowerPolicyServiceType,
) {
    power_policy_service::service::task::psu_task(psu_events, power_policy).await;
}

#[embassy_executor::task]
async fn type_c_service_task(
    service: &'static Mutex<GlobalRawMutex, TypeCServiceType>,
    event_receiver: TypeCServiceEventReceiverType,
) {
    type_c_service::task::task(service, event_receiver).await;
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

    info!("Spawining PD controller task");
    static CONTROLLER_MUTEX: StaticCell<Tps6699xMutex<'_>> = StaticCell::new();
    let controller_mutex = CONTROLLER_MUTEX.init(Mutex::new(tps6699x_drv::tps66994(
        tps6699x,
        Default::default(),
        Default::default(),
        "tps6699x_0",
    )));

    define_controller_port_static_cell_channel!(pub(self), port0, GlobalRawMutex, Tps6699xMutex<'static>);
    let PortComponents {
        port: port0,
        power_policy_receiver: policy_receiver0,
        event_receiver: event_receiver0,
        interrupt_sender: port0_interrupt_sender,
        type_c_receiver: type_c_receiver0,
    } = port0::create("PD0", LocalPortId(0), Default::default(), controller_mutex);

    define_controller_port_static_cell_channel!(pub(self), port1, GlobalRawMutex, Tps6699xMutex<'static>);
    let PortComponents {
        port: port1,
        power_policy_receiver: policy_receiver1,
        event_receiver: event_receiver1,
        interrupt_sender: port1_interrupt_sender,
        type_c_receiver: type_c_receiver1,
    } = port1::create("PD1", LocalPortId(1), Default::default(), controller_mutex);

    let port_event_splitter = PortEventSplitter::new([port0_interrupt_sender, port1_interrupt_sender]);

    // The service is the only receiver and we only use a DynImmediatePublisher, which doesn't take a publisher slot
    static POWER_POLICY_CHANNEL: StaticCell<
        PubSubChannel<GlobalRawMutex, power_policy_interface::service::event::EventData, 4, 1, 0>,
    > = StaticCell::new();

    let power_policy_channel = POWER_POLICY_CHANNEL.init(PubSubChannel::new());
    let power_policy_sender: PowerPolicySenderType =
        MapSender::new(power_policy_channel.dyn_immediate_publisher(), |e| e.into());
    // Guaranteed to not panic since we initialized the channel above
    let power_policy_subscriber = power_policy_channel.dyn_subscriber().unwrap();

    // Create power policy service
    let power_policy_registration = ArrayRegistration {
        psus: [port0, port1],
        chargers: [],
        service_notifiers: [power_policy_sender.into()],
    };

    static POWER_SERVICE: StaticCell<PowerPolicyServiceType> = StaticCell::new();
    let power_service = POWER_SERVICE.init(Mutex::new(power_policy_service::service::Service::new(
        power_policy_registration,
        power_policy_service::service::config::Config::default(),
    )));

    static TYPE_C_SERVICE: StaticCell<Mutex<GlobalRawMutex, TypeCServiceType>> = StaticCell::new();
    let type_c_service = TYPE_C_SERVICE.init(Mutex::new(Service::new(
        Default::default(),
        TypeCRegistrationType {
            ports: [port0, port1],
            service_senders: [NoopSender],
            port_data: [
                PortData {
                    local_port: Some(LocalPortId(0)),
                },
                PortData {
                    local_port: Some(LocalPortId(1)),
                },
            ],
        },
    )));

    info!("Spawining type-c service task");
    spawner.spawn(
        type_c_service_task(
            type_c_service,
            TypeCServiceEventReceiverType::new(
                [port0, port1],
                [type_c_receiver0, type_c_receiver1],
                power_policy_subscriber,
            ),
        )
        .expect("Failed to create type-c service task"),
    );

    info!("Spawining power policy task");
    spawner.spawn(
        power_policy_task(
            PsuEventReceivers::new([port0, port1], [policy_receiver0, policy_receiver1]),
            power_service,
        )
        .expect("Failed to create power policy task"),
    );

    spawner.spawn(port_task(event_receiver0, port0).expect("Failed to create controller0 task"));

    spawner.spawn(port_task(event_receiver1, port1).expect("Failed to create controller1 task"));

    spawner.spawn(
        interrupt_splitter_task(interrupt_receiver, port_event_splitter)
            .expect("Failed to spawn interrupt splitter task"),
    );
}
