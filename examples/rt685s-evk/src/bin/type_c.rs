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
use embassy_time::{self as _, Delay};
use embedded_cfu_protocol::protocol_definitions::{FwUpdateOffer, FwUpdateOfferResponse, FwVersion, HostToken};
use embedded_services::GlobalRawMutex;
use embedded_services::power::policy::DeviceId as PowerId;
use embedded_services::type_c::{self, Cached, ControllerId};
use embedded_services::{error, info};
use embedded_usb_pd::GlobalPortId;
use static_cell::StaticCell;
use tps6699x::asynchronous::embassy as tps6699x;
use type_c_service::driver::tps6699x::{self as tps6699x_driver, Tps6699xWrapper};
use type_c_service::wrapper::backing::{ReferencedStorage, Storage};

extern crate rt685s_evk_example;

const CONTROLLER0_ID: ControllerId = ControllerId(0);
const PORT0_ID: GlobalPortId = GlobalPortId(0);
const PORT1_ID: GlobalPortId = GlobalPortId(1);
const PORT0_PWR_ID: PowerId = PowerId(0);
const PORT1_PWR_ID: PowerId = PowerId(1);

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
type Wrapper<'a> = Tps6699xWrapper<'a, GlobalRawMutex, BusDevice<'a>, Validator>;
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

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_imxrt::init(Default::default());

    info!("Embedded service init");
    embedded_services::init().await;

    type_c::controller::init();

    info!("Spawining power policy task");
    spawner.must_spawn(power_policy_service::task(Default::default()));

    info!("Spawining type-c service task");
    spawner.must_spawn(type_c_service::task(Default::default()));

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

    static STORAGE: StaticCell<Storage<TPS66994_NUM_PORTS, GlobalRawMutex>> = StaticCell::new();
    let storage = STORAGE.init(Storage::new(
        CONTROLLER0_ID,
        0, // CFU component ID
        [(PORT0_ID, PORT0_PWR_ID), (PORT1_ID, PORT1_PWR_ID)],
    ));

    static REFERENCED: StaticCell<ReferencedStorage<TPS66994_NUM_PORTS, GlobalRawMutex>> = StaticCell::new();
    let referenced = REFERENCED.init(storage.create_referenced());

    info!("Spawining PD controller task");
    static PD_CONTROLLER: StaticCell<Wrapper> = StaticCell::new();
    let pd_controller =
        PD_CONTROLLER.init(tps6699x_driver::tps66994(tps6699x, referenced, Default::default(), Validator).unwrap());

    pd_controller.register().await.unwrap();
    spawner.must_spawn(pd_controller_task(pd_controller));

    // Sync our internal state with the hardware
    type_c::external::sync_controller_state(CONTROLLER0_ID).await.unwrap();

    embassy_time::Timer::after_secs(10).await;

    let status = type_c::external::get_controller_status(CONTROLLER0_ID).await.unwrap();

    info!("Controller status: {:?}", status);

    let status = type_c::external::get_port_status(PORT0_ID, Cached(true)).await.unwrap();
    info!("Port status: {:?}", status);

    let status = type_c::external::get_port_status(PORT1_ID, Cached(true)).await.unwrap();
    info!("Port status: {:?}", status);
}
