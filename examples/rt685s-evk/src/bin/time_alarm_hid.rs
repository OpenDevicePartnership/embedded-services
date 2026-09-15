#![no_std]
#![no_main]

use embassy_imxrt::i2c::slave::{Address, I2cSlave};
use embassy_imxrt::i2c::{self, Async};
use embassy_imxrt::{bind_interrupts, peripherals};
use embedded_mcu_hal::nvram::Nvram;
use embedded_services::info;
use static_cell::StaticCell;
use time_alarm_service_interface::TimeAlarmService;
use {defmt_rtt as _, panic_probe as _};

const SLAVE_ADDR: Option<Address> = Address::new(0x15);

bind_interrupts!(struct Irqs {
    FLEXCOMM2 => i2c::InterruptHandler<peripherals::FLEXCOMM2>;
});

type TimeAlarmServiceType = time_alarm_service::Service<'static>;
type TimeAlarmServiceRelayHandlerType = time_alarm_service_relay::hid::TimeAlarmHidRelay<'static, TimeAlarmServiceType>;

#[embassy_executor::main]
async fn main(spawner: embassy_executor::Spawner) {
    let p = embassy_imxrt::init(Default::default());

    static RTC: StaticCell<embassy_imxrt::rtc::Rtc> = StaticCell::new();
    let rtc = RTC.init(embassy_imxrt::rtc::Rtc::new(p.RTC));
    let (dt_clock, rtc_nvram) = rtc.split();

    let [tz, ac_expiration, ac_policy, dc_expiration, dc_policy, ..] = rtc_nvram.storage();

    embedded_services::init().await;
    info!("services initialized");

    let time_service = odp_service_common::spawn_service!(spawner, TimeAlarmServiceType, |resources| {
        time_alarm_service::Service::new(
            resources,
            dt_clock,
            tz,
            ac_expiration,
            ac_policy,
            dc_expiration,
            dc_policy,
        )
    })
    .expect("Failed to spawn time alarm service");

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

    let _hidsvc = odp_service_common::spawn_service!(
        spawner,
        hidi2c_target_service::Service<
            'static,
            I2cSlave<'static, Async>,
            gpio::Output<'static>,
            TimeAlarmServiceRelayHandlerType,
        >,
        |resources| hidi2c_target_service::Service::new(
            resources,
            i2c,
            attn_pin,
            TimeAlarmServiceRelayHandlerType::new(time_service),
            hidi2c_target_service::HardwareVersionInfo {
                vendor_id: hidi2c_target_service::VendorId::new(0x3333).unwrap(),
                product_id: hidi2c_target_service::ProductId(0x4444),
                version_id: hidi2c_target_service::VersionId(0x0001),
            },
            hidi2c_target_service::TimeoutSettings::default()
        )
    )
    .expect("Failed to spawn HID service");

    loop {
        embassy_time::Timer::after(embassy_time::Duration::from_secs(10)).await;
        info!("Current time from service: {:?}", time_service.get_real_time().unwrap());
    }
}
