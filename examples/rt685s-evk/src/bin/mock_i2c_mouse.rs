#![no_std]
#![no_main]

use defmt::info;
use defmt_rtt as _;
use embassy_executor::Spawner;
use embassy_imxrt::i2c::slave::{Address, I2cSlave};
use embassy_imxrt::i2c::{self, Async};
use embassy_imxrt::{bind_interrupts, peripherals};
use panic_probe as _;
use rt685s_evk_example::mocks::mouse::{MockMouseHidRelay, MockMouseResources, MockMouseService};
use static_cell::StaticCell;

const SLAVE_ADDR: Option<Address> = Address::new(0x15);

bind_interrupts!(struct Irqs {
    FLEXCOMM2 => i2c::InterruptHandler<peripherals::FLEXCOMM2>;
});

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_imxrt::init(Default::default());
    info!("HID-I2C mock mouse example starting...");
    let i2c = I2cSlave::new_async(p.FLEXCOMM2, p.PIO0_18, p.PIO0_17, Irqs, SLAVE_ADDR.unwrap(), p.DMA0_CH4).unwrap();

    // GPIO on P0_28.
    use embassy_imxrt::gpio;
    let attn_pin = gpio::Output::new(
        p.PIO0_28,
        gpio::Level::High,
        gpio::DriveMode::OpenDrain,
        gpio::DriveStrength::Normal,
        gpio::SlewRate::Standard,
    );

    static MOUSE_RESOURCES: StaticCell<MockMouseResources> = StaticCell::new();
    let mouse_resources = MOUSE_RESOURCES.init(MockMouseResources::default());
    let (mouse_service, mut mouse_runner) = MockMouseService::new(mouse_resources);

    // NOTE: here's where the "aggregate HID devices" macro is currently missing.  Compare with time_alarm.rs where we do this:
    //
    //     impl_odp_mctp_relay_handler!(
    //         EspiRelayHandler;
    //         TimeAlarm, 0x0B, crate::TimeAlarmServiceRelayHandlerType;
    //     );
    //
    //  We will eventually write a macro that looks something like this:
    //
    //     impl_hid_aggregate_device!(
    //         MyAggregateDevice;
    //         time_alarm_service_relay::hid::TimeAlarmHidRelay,
    //         battery_service_relay::hid::BatteryHidRelay,
    //         ...
    //     );
    //
    // which will emit a type MyAggregateDevice that implements HidDevice and takes as construction parameters one instance
    // of each of the handlers, in the order they were specified.
    //
    // For a concrete example of this pattern, see what we're doing with MCTP here: https://github.com/OpenDevicePartnership/odp-embedded-controller/blob/d6fb3ce5d9ae52ca51d6ef7b87518c6c4cb3c809/platform/platform-common/src/lib.rs#L20
    //
    // This depends on writing the 'hid support library', though, so for now we're just going to directly use the time-alarm device for testing
    // (pending getting actual hardware to test on, implementing I2C traits for embassy, etc).

    let _hidsvc = odp_service_common::spawn_service!(
        spawner,
        hidi2c_target_service::Service<
            'static,
            I2cSlave<'static, Async>,
            gpio::Output<'static>,
            MockMouseHidRelay<'static>,
        >,
        |resources| hidi2c_target_service::Service::new(
            resources,
            i2c,
            attn_pin,
            MockMouseHidRelay::new(mouse_service),
            hidi2c_target_service::HardwareVersionInfo {
                vendor_id: hidi2c_target_service::VendorId::new(0x1234).unwrap(),
                product_id: hidi2c_target_service::ProductId(0x5678),
                version_id: hidi2c_target_service::VersionId(0x0001),
            },
            hidi2c_target_service::TimeoutSettings::default()
        )
    )
    .expect("Failed to spawn HID service");

    info!("Waiting 10s before starting to send inputs");
    embassy_time::Timer::after(embassy_time::Duration::from_secs(10)).await;

    loop {
        info!("clicking mouse");
        mouse_runner.send_click().await;
        embassy_time::Timer::after(embassy_time::Duration::from_millis(2000)).await;
    }
}
