#![no_std]
#![no_main]

use ::tps6699x::ADDR0;
use defmt::info;
use embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice;
use embassy_executor::Spawner;
use embassy_imxrt::gpio::{Input, Inverter, Pull};
use embassy_imxrt::i2c::master::{I2cMaster, Speed};
use embassy_imxrt::i2c::Async;
use embassy_imxrt::{bind_interrupts, peripherals};
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::mutex::Mutex;
use embassy_sync::once_lock::OnceLock;
use embassy_time as _;
use embedded_services::comms;
use embedded_services::power::policy::DeviceId as PowerId;
use embedded_services::type_c::{self, ControllerId, GlobalPortId};
use static_cell::StaticCell;
use tps6699x::asynchronous::embassy as tps6699x;
use type_c_service::driver::tps6699x::{self as tps6699x_driver, Tps66994Wrapper};

extern crate embedded_services_examples;

const CONTROLLER0_ID: ControllerId = ControllerId(0);
const PORT0_ID: GlobalPortId = GlobalPortId(0);
const PORT1_ID: GlobalPortId = GlobalPortId(1);
const PORT0_PWR_ID: PowerId = PowerId(0);
const PORT1_PWR_ID: PowerId = PowerId(1);

bind_interrupts!(struct Irqs {
    FLEXCOMM2 => embassy_imxrt::i2c::InterruptHandler<peripherals::FLEXCOMM2>;
});

type BusMaster<'a> = I2cMaster<'a, Async>;
type BusDevice<'a> = I2cDevice<'a, NoopRawMutex, BusMaster<'a>>;
type Wrapper<'a> = Tps66994Wrapper<'a, NoopRawMutex, BusDevice<'a>>;
type Controller<'a> = tps6699x::controller::Controller<NoopRawMutex, BusDevice<'a>>;
type Interrupt<'a> = tps6699x::Interrupt<'a, NoopRawMutex, BusDevice<'a>>;

/// Battery mock that receives messages from power policy
mod battery {
    use defmt::{info, trace};
    use embedded_services::comms;
    use embedded_services::power::policy;

    pub struct Device {
        pub tp: comms::Endpoint,
    }

    impl Device {
        pub fn new() -> Self {
            Self {
                tp: comms::Endpoint::uninit(comms::EndpointID::Internal(comms::Internal::Battery)),
            }
        }
    }

    impl comms::MailboxDelegate for Device {
        fn receive(&self, message: &comms::Message) {
            trace!("Got message");
            if let Some(message) = message.data.get::<policy::CommsMessage>() {
                match message.data {
                    policy::CommsData::ConsumerDisconnected(id) => {
                        info!("Consumer disconnected: {}", id.0);
                    }
                    policy::CommsData::ConsumerConnected(id, capability) => {
                        info!("Consumer connected: {} {:?}", id.0, capability);
                    }
                }
            }
        }
    }
}

/// Debug accesory listener mock
mod debug {
    use defmt::{info, trace};
    use embedded_services::comms;
    use embedded_services::type_c;

    pub struct Device {
        pub tp: comms::Endpoint,
    }

    impl Device {
        pub fn new() -> Self {
            Self {
                tp: comms::Endpoint::uninit(comms::EndpointID::Internal(comms::Internal::Usbc)),
            }
        }
    }

    impl comms::MailboxDelegate for Device {
        fn receive(&self, message: &comms::Message) {
            trace!("Got message");
            if let Some(message) = message.data.get::<type_c::comms::DebugAccessoryMessage>() {
                info!("Debug accessory message: {:?}", message);
            }
        }
    }
}

#[embassy_executor::task]
async fn pd_controller_task(controller: &'static Wrapper<'static>) {
    loop {
        controller.process().await;
    }
}

#[embassy_executor::task]
async fn interrupt_task(mut int_in: Input<'static>, mut interrupt: Interrupt<'static>) {
    tps6699x::task::interrupt_task(&mut int_in, [&mut interrupt]).await;
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_imxrt::init(Default::default());

    info!("Embedded service init");
    embedded_services::init().await;

    type_c::controller::init();

    info!("Spawining power policy task");
    spawner.must_spawn(power_policy_service::task());

    info!("Spawining type-c service task");
    spawner.must_spawn(type_c_service::task());

    let int_in = Input::new(p.PIO1_7, Pull::Up, Inverter::Disabled);
    static BUS: OnceLock<Mutex<NoopRawMutex, BusMaster<'static>>> = OnceLock::new();
    let bus = BUS.get_or_init(|| {
        Mutex::new(I2cMaster::new_async(p.FLEXCOMM2, p.PIO0_18, p.PIO0_17, Irqs, Speed::Standard, p.DMA0_CH5).unwrap())
    });

    let device = I2cDevice::new(bus);

    static CONTROLLER: StaticCell<Controller<'static>> = StaticCell::new();
    let controller = CONTROLLER.init(Controller::new_tps66994(device, ADDR0).unwrap());
    let (tps6699x, interrupt) = controller.make_parts();

    info!("Spawining interrupt task");
    spawner.must_spawn(interrupt_task(int_in, interrupt));

    info!("Spawining PD controller task");
    static PD_CONTROLLER: OnceLock<Wrapper> = OnceLock::new();
    let pd_controller = PD_CONTROLLER.get_or_init(|| {
        tps6699x_driver::tps66994(
            tps6699x,
            CONTROLLER0_ID,
            [PORT0_ID, PORT1_ID],
            [PORT0_PWR_ID, PORT1_PWR_ID],
        )
        .unwrap()
    });

    pd_controller.register().await.unwrap();
    spawner.must_spawn(pd_controller_task(pd_controller));

    static BATTERY: OnceLock<battery::Device> = OnceLock::new();
    let battery = BATTERY.get_or_init(|| battery::Device::new());

    comms::register_endpoint(battery, &battery.tp).await.unwrap();

    static DEBUG_ACCESSORY: OnceLock<debug::Device> = OnceLock::new();
    let debug_accessory = DEBUG_ACCESSORY.get_or_init(|| debug::Device::new());
    comms::register_endpoint(debug_accessory, &debug_accessory.tp)
        .await
        .unwrap();
}
