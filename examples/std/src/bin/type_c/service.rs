use embassy_executor::{Executor, Spawner};
use embassy_sync::channel::{DynamicReceiver, DynamicSender};
use embassy_sync::mutex::Mutex;
use embassy_sync::pubsub::{DynImmediatePublisher, DynSubscriber, PubSubChannel};
use embassy_time::Timer;
use embedded_services::GlobalRawMutex;
use embedded_services::event::{MapSender, NoopSender};
use embedded_usb_pd::LocalPortId;
use embedded_usb_pd::ado::Ado;
use embedded_usb_pd::type_c::Current;
use log::*;
use power_policy_interface::charger::mock::ChargerType;
use power_policy_interface::psu;
use power_policy_service::psu::PsuEventReceivers;
use power_policy_service::service::registration::ArrayRegistration;
use static_cell::StaticCell;
use std_examples::type_c::mock_controller::Port;
use std_examples::type_c::mock_controller::{self, InterruptReceiver};
use type_c_interface::port::event::PortEventBitfield;
use type_c_interface::service::event::PortEventData as ServicePortEventData;
use type_c_service::controller::event_receiver::InterruptReceiver as _;
use type_c_service::controller::event_receiver::{EventReceiver as PortEventReceiver, PortEventSplitter};
use type_c_service::controller::macros::PortComponents;
use type_c_service::controller::state::SharedState;
use type_c_service::define_controller_port_static_cell_channel;
use type_c_service::service::Service;
use type_c_service::service::config::Config;
use type_c_service::util::power_capability_from_current;

const DELAY_MS: u64 = 1000;

type ControllerType = Mutex<GlobalRawMutex, mock_controller::Controller<'static>>;
type PortType = Mutex<GlobalRawMutex, Port<'static>>;

type PowerPolicySenderType = MapSender<
    power_policy_interface::service::event::Event<'static, PortType>,
    power_policy_interface::service::event::EventData,
    DynImmediatePublisher<'static, power_policy_interface::service::event::EventData>,
    fn(
        power_policy_interface::service::event::Event<'static, PortType>,
    ) -> power_policy_interface::service::event::EventData,
>;

type PowerPolicyReceiverType = DynSubscriber<'static, power_policy_interface::service::event::EventData>;

type PowerPolicyServiceType = Mutex<
    GlobalRawMutex,
    power_policy_service::service::Service<
        'static,
        ArrayRegistration<'static, PortType, 1, PowerPolicySenderType, 1, ChargerType, 0>,
    >,
>;

const PORT_COUNT: usize = 1;
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

type ServiceType = type_c_service::service::Service<'static, TypeCRegistrationType>;
type SharedStateType = Mutex<GlobalRawMutex, SharedState>;
type PortEventReceiverType = PortEventReceiver<
    'static,
    SharedStateType,
    DynamicReceiver<'static, PortEventBitfield>,
    DynamicReceiver<'static, type_c_service::controller::event::Loopback>,
>;

#[embassy_executor::task]
async fn port_task(mut event_receiver: PortEventReceiverType, port: &'static PortType) {
    loop {
        let event = event_receiver.wait_event().await;
        let output = port.lock().await.process_event(event).await;
        if let Err(e) = output {
            error!("Error processing event: {e:?}");
        }

        let output = output.unwrap();
        if let Some(ServicePortEventData::Alert(ado)) = &output {
            info!("PD alert received: {:?}", ado);
        }
    }
}

#[embassy_executor::task]
async fn interrupt_splitter_task(
    mut interrupt_receiver: InterruptReceiver<'static>,
    mut interrupt_splitter: PortEventSplitter<1, DynamicSender<'static, PortEventBitfield>>,
) -> ! {
    loop {
        let interrupts = interrupt_receiver.wait_interrupt().await;
        interrupt_splitter.process_interrupts(interrupts).await;
    }
}

#[embassy_executor::task]
async fn task(spawner: Spawner) {
    embedded_services::init().await;

    static STATE: StaticCell<mock_controller::ControllerState> = StaticCell::new();
    let state = STATE.init(mock_controller::ControllerState::new());

    static CONTROLLER: StaticCell<ControllerType> = StaticCell::new();
    let controller = CONTROLLER.init(Mutex::new(mock_controller::Controller::new(state, "Controller0")));

    define_controller_port_static_cell_channel!(pub(self), port, GlobalRawMutex, Mutex<GlobalRawMutex, mock_controller::Controller<'static>>);
    let PortComponents {
        port,
        power_policy_receiver,
        event_receiver,
        interrupt_sender: port_interrupt_sender,
        type_c_receiver,
    } = port::create("PD0", LocalPortId(0), Default::default(), controller);

    // Create type-c service
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
        psus: [port],
        service_senders: [power_policy_sender],
        chargers: [],
    };

    static POWER_SERVICE: StaticCell<PowerPolicyServiceType> = StaticCell::new();
    let power_service = POWER_SERVICE.init(Mutex::new(power_policy_service::service::Service::new(
        power_policy_registration,
        power_policy_service::service::config::Config::default(),
    )));

    static TYPE_C_SERVICE: StaticCell<Mutex<GlobalRawMutex, ServiceType>> = StaticCell::new();
    let type_c_service = TYPE_C_SERVICE.init(Mutex::new(Service::new(
        Config::default(),
        type_c_service::service::registration::ArrayRegistration {
            ports: [port],
            service_senders: [NoopSender],
            port_data: [type_c_service::service::registration::PortData {
                local_port: Some(LocalPortId(0)),
            }],
        },
    )));

    // Spin up power policy service
    spawner.spawn(
        power_policy_psu_task(PsuEventReceivers::new([port], [power_policy_receiver]), power_service)
            .expect("Failed to create power policy task"),
    );
    spawner.spawn(
        type_c_service_task(
            type_c_service,
            TypeCServiceEventReceiverType::new([port], [type_c_receiver], power_policy_subscriber),
        )
        .expect("Failed to create type-c service task"),
    );

    spawner.spawn(port_task(event_receiver, port).expect("Failed to create controller task"));

    spawner.spawn(
        interrupt_splitter_task(
            state.create_interrupt_receiver(),
            PortEventSplitter::new([port_interrupt_sender]),
        )
        .expect("Failed to create interrupt splitter task"),
    );

    Timer::after_millis(1000).await;
    info!("Simulating connection");
    state
        .connect_sink(power_capability_from_current(Current::UsbDefault), false)
        .await;
    Timer::after_millis(DELAY_MS).await;

    info!("Simulating PD alert");
    state.send_pd_alert(Ado::PowerButtonPress).await;
    Timer::after_millis(DELAY_MS).await;

    info!("Simulating disconnection");
    state.disconnect().await;
    Timer::after_millis(DELAY_MS).await;

    info!("Simulating debug accessory connection");
    state.connect_debug_accessory_source(Current::UsbDefault).await;
    Timer::after_millis(DELAY_MS).await;

    info!("Simulating debug accessory disconnection");
    state.disconnect().await;
    Timer::after_millis(DELAY_MS).await;
}

#[embassy_executor::task]
async fn power_policy_psu_task(
    psu_events: PsuEventReceivers<'static, 1, PortType, DynamicReceiver<'static, psu::event::EventData>>,
    power_policy: &'static PowerPolicyServiceType,
) {
    power_policy_service::service::task::psu_task(psu_events, power_policy).await;
}

#[embassy_executor::task]
async fn type_c_service_task(
    service: &'static Mutex<GlobalRawMutex, ServiceType>,
    event_receiver: TypeCServiceEventReceiverType,
) {
    info!("Starting type-c task");
    type_c_service::task::task(service, event_receiver).await;
}

fn main() {
    env_logger::builder().filter_level(log::LevelFilter::Trace).init();

    static EXECUTOR: StaticCell<Executor> = StaticCell::new();
    let executor = EXECUTOR.init(Executor::new());

    executor.run(|spawner| {
        spawner.spawn(task(spawner).expect("Failed to create task"));
    });
}
