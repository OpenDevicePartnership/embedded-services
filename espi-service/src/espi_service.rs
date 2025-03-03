use core::cell::RefCell;

use embassy_sync::blocking_mutex::raw::ThreadModeRawMutex;
use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::once_lock::OnceLock;
use embedded_services::comms::{self, EndpointID, External};
use embedded_services::{ec_type, error, info};

pub struct Service {
    pub endpoint: comms::Endpoint,
}

impl Service {
    pub fn new() -> Self {
        Service {
            endpoint: comms::Endpoint::uninit(EndpointID::External(External::Host)),
        }
    }
}

impl Default for Service {
    fn default() -> Self {
        Self::new()
    }
}

impl comms::MailboxDelegate for Service {
    fn receive(&self, message: &comms::Message) {
        if let Some(msg) = message.data.get::<ec_type::message::CapabilitiesMessage>() {
            update_capabilities_section(msg);
        } else if let Some(msg) = message.data.get::<ec_type::message::BatteryMessage>() {
            update_battery_section(msg);
        } else if let Some(msg) = message.data.get::<ec_type::message::ThermalMessage>() {
            update_thermal_section(msg);
        } else if let Some(msg) = message.data.get::<ec_type::message::TimeAlarmMessage>() {
            update_time_alarm_section(msg);
        }
    }
}

static ESPI_SERVICE: OnceLock<Service> = OnceLock::new();
static MEMORY_MAP: OnceLock<Mutex<ThreadModeRawMutex, RefCell<&mut ec_type::structure::ECMemory>>> = OnceLock::new();

// Initialize eSPI service and register it with the transport service
async fn init() {
    info!("Initializing memory map");
    MEMORY_MAP.try_get().unwrap().lock(|memory_map| {
        let mut memory_map = memory_map.borrow_mut();
        memory_map.ver.major = 0;
        memory_map.ver.minor = 1;
        memory_map.ver.spin = 0;
        memory_map.ver.res0 = 0;
    });

    let espi_service = ESPI_SERVICE.get_or_init(Service::new);

    comms::register_endpoint(espi_service, &espi_service.endpoint)
        .await
        .unwrap();
}

fn update_capabilities_section(msg: &ec_type::message::CapabilitiesMessage) {
    MEMORY_MAP.try_get().unwrap().lock(|memory_map| {
        let mut memory_map = memory_map.borrow_mut();
        match msg {
            ec_type::message::CapabilitiesMessage::Events(events) => memory_map.caps.events = *events,
            ec_type::message::CapabilitiesMessage::FwVersion(fw_version) => memory_map.caps.fw_version = *fw_version,
            ec_type::message::CapabilitiesMessage::SecureState(secure_state) => {
                memory_map.caps.secure_state = *secure_state
            }
            ec_type::message::CapabilitiesMessage::BootStatus(boot_status) => {
                memory_map.caps.boot_status = *boot_status
            }
            ec_type::message::CapabilitiesMessage::FanMask(fan_mask) => memory_map.caps.fan_mask = *fan_mask,
            ec_type::message::CapabilitiesMessage::BatteryMask(battery_mask) => {
                memory_map.caps.battery_mask = *battery_mask
            }
            ec_type::message::CapabilitiesMessage::TempMask(temp_mask) => memory_map.caps.temp_mask = *temp_mask,
            ec_type::message::CapabilitiesMessage::KeyMask(key_mask) => memory_map.caps.key_mask = *key_mask,
            ec_type::message::CapabilitiesMessage::DebugMask(debug_mask) => memory_map.caps.debug_mask = *debug_mask,
        }
    });
}

fn update_battery_section(msg: &ec_type::message::BatteryMessage) {
    MEMORY_MAP.try_get().unwrap().lock(|memory_map| {
        let mut memory_map = memory_map.borrow_mut();
        match msg {
            ec_type::message::BatteryMessage::Events(events) => memory_map.batt.events = *events,
            ec_type::message::BatteryMessage::LastFullCharge(last_full_charge) => {
                memory_map.batt.last_full_charge = *last_full_charge
            }
            ec_type::message::BatteryMessage::CycleCount(cycle_count) => memory_map.batt.cycle_count = *cycle_count,
            ec_type::message::BatteryMessage::State(state) => memory_map.batt.state = *state,
            ec_type::message::BatteryMessage::PresentRate(present_rate) => memory_map.batt.present_rate = *present_rate,
            ec_type::message::BatteryMessage::RemainCap(remain_cap) => memory_map.batt.remain_cap = *remain_cap,
            ec_type::message::BatteryMessage::PresentVolt(present_volt) => memory_map.batt.present_volt = *present_volt,
            ec_type::message::BatteryMessage::PsrState(psr_state) => memory_map.batt.psr_state = *psr_state,
            ec_type::message::BatteryMessage::PsrMaxOut(psr_max_out) => memory_map.batt.psr_max_out = *psr_max_out,
            ec_type::message::BatteryMessage::PsrMaxIn(psr_max_in) => memory_map.batt.psr_max_in = *psr_max_in,
            ec_type::message::BatteryMessage::PeakLevel(peek_level) => memory_map.batt.peak_level = *peek_level,
            ec_type::message::BatteryMessage::PeakPower(peek_power) => memory_map.batt.peak_power = *peek_power,
            ec_type::message::BatteryMessage::SusLevel(sus_level) => memory_map.batt.sus_level = *sus_level,
            ec_type::message::BatteryMessage::SusPower(sus_power) => memory_map.batt.sus_power = *sus_power,
            ec_type::message::BatteryMessage::PeakThres(peek_thres) => memory_map.batt.peak_thres = *peek_thres,
            ec_type::message::BatteryMessage::SusThres(sus_thres) => memory_map.batt.sus_thres = *sus_thres,
            ec_type::message::BatteryMessage::TripThres(trip_thres) => memory_map.batt.trip_thres = *trip_thres,
            ec_type::message::BatteryMessage::BmcData(bmc_data) => memory_map.batt.bmc_data = *bmc_data,
            ec_type::message::BatteryMessage::BmdData(bmd_data) => memory_map.batt.bmd_data = *bmd_data,
            ec_type::message::BatteryMessage::BmdFlags(bmd_flags) => memory_map.batt.bmd_flags = *bmd_flags,
            ec_type::message::BatteryMessage::BmdCount(bmd_count) => memory_map.batt.bmd_count = *bmd_count,
            ec_type::message::BatteryMessage::ChargeTime(charge_time) => memory_map.batt.charge_time = *charge_time,
            ec_type::message::BatteryMessage::RunTime(run_time) => memory_map.batt.run_time = *run_time,
            ec_type::message::BatteryMessage::SampleTime(sample_time) => memory_map.batt.sample_time = *sample_time,
        }
    });
}

fn update_thermal_section(msg: &ec_type::message::ThermalMessage) {
    MEMORY_MAP.try_get().unwrap().lock(|memory_map| {
        let mut memory_map = memory_map.borrow_mut();
        match msg {
            ec_type::message::ThermalMessage::Events(events) => memory_map.therm.events = *events,
            ec_type::message::ThermalMessage::CoolMode(cool_mode) => memory_map.therm.cool_mode = *cool_mode,
            ec_type::message::ThermalMessage::DbaLimit(dba_limit) => memory_map.therm.dba_limit = *dba_limit,
            ec_type::message::ThermalMessage::SonneLimit(sonne_limit) => memory_map.therm.sonne_limit = *sonne_limit,
            ec_type::message::ThermalMessage::MaLimit(ma_limit) => memory_map.therm.ma_limit = *ma_limit,
            ec_type::message::ThermalMessage::Fan1OnTemp(fan1_on_temp) => memory_map.therm.fan1_on_temp = *fan1_on_temp,
            ec_type::message::ThermalMessage::Fan1RampTemp(fan1_ramp_temp) => {
                memory_map.therm.fan1_ramp_temp = *fan1_ramp_temp
            }
            ec_type::message::ThermalMessage::Fan1MaxTemp(fan1_max_temp) => {
                memory_map.therm.fan1_max_temp = *fan1_max_temp
            }
            ec_type::message::ThermalMessage::Fan1CrtTemp(fan1_crt_temp) => {
                memory_map.therm.fan1_crt_temp = *fan1_crt_temp
            }
            ec_type::message::ThermalMessage::Fan1HotTemp(fan1_hot_temp) => {
                memory_map.therm.fan1_hot_temp = *fan1_hot_temp
            }
            ec_type::message::ThermalMessage::Fan1MaxRpm(fan1_max_rpm) => memory_map.therm.fan1_max_rpm = *fan1_max_rpm,
            ec_type::message::ThermalMessage::Fan1CurRpm(fan1_cur_rpm) => memory_map.therm.fan1_cur_rpm = *fan1_cur_rpm,
            ec_type::message::ThermalMessage::Tmp1Val(tmp1_val) => memory_map.therm.tmp1_val = *tmp1_val,
            ec_type::message::ThermalMessage::Tmp1Timeout(tmp1_timeout) => {
                memory_map.therm.tmp1_timeout = *tmp1_timeout
            }
            ec_type::message::ThermalMessage::Tmp1Low(tmp1_low) => memory_map.therm.tmp1_low = *tmp1_low,
            ec_type::message::ThermalMessage::Tmp1High(tmp1_high) => memory_map.therm.tmp1_high = *tmp1_high,
        }
    });
}

fn update_time_alarm_section(msg: &ec_type::message::TimeAlarmMessage) {
    MEMORY_MAP.try_get().unwrap().lock(|memory_map| {
        let mut memory_map = memory_map.borrow_mut();
        match msg {
            ec_type::message::TimeAlarmMessage::Events(events) => memory_map.alarm.events = *events,
            ec_type::message::TimeAlarmMessage::Capability(capability) => memory_map.alarm.capability = *capability,
            ec_type::message::TimeAlarmMessage::Year(year) => memory_map.alarm.year = *year,
            ec_type::message::TimeAlarmMessage::Month(month) => memory_map.alarm.month = *month,
            ec_type::message::TimeAlarmMessage::Day(day) => memory_map.alarm.day = *day,
            ec_type::message::TimeAlarmMessage::Hour(hour) => memory_map.alarm.hour = *hour,
            ec_type::message::TimeAlarmMessage::Minute(minute) => memory_map.alarm.minute = *minute,
            ec_type::message::TimeAlarmMessage::Second(second) => memory_map.alarm.second = *second,
            ec_type::message::TimeAlarmMessage::Valid(valid) => memory_map.alarm.valid = *valid,
            ec_type::message::TimeAlarmMessage::Daylight(daylight) => memory_map.alarm.daylight = *daylight,
            ec_type::message::TimeAlarmMessage::Res1(res1) => memory_map.alarm.res1 = *res1,
            ec_type::message::TimeAlarmMessage::Milli(milli) => memory_map.alarm.milli = *milli,
            ec_type::message::TimeAlarmMessage::TimeZone(time_zone) => memory_map.alarm.time_zone = *time_zone,
            ec_type::message::TimeAlarmMessage::Res2(res2) => memory_map.alarm.res2 = *res2,
            ec_type::message::TimeAlarmMessage::AlarmStatus(alarm_status) => {
                memory_map.alarm.alarm_status = *alarm_status
            }
            ec_type::message::TimeAlarmMessage::AcTimeVal(ac_time_val) => memory_map.alarm.ac_time_val = *ac_time_val,
            ec_type::message::TimeAlarmMessage::DcTimeVal(dc_time_val) => memory_map.alarm.dc_time_val = *dc_time_val,
        }
    });
}

use embassy_imxrt::espi;

#[embassy_executor::task]
pub async fn espi_service(mut espi: espi::Espi<'static>, memory_map_buffer: &'static mut [u8]) {
    info!("Reserved eSPI memory map buffer size: {}", memory_map_buffer.len());
    info!("eSPI MemoryMap size: {}", size_of::<ec_type::structure::ECMemory>());

    if size_of::<ec_type::structure::ECMemory>() > memory_map_buffer.len() {
        panic!("eSPI MemoryMap is too big for reserved memory buffer!!!");
    }

    memory_map_buffer.fill(0);

    let memory_map: &mut ec_type::structure::ECMemory =
        unsafe { &mut *(memory_map_buffer.as_mut_ptr() as *mut ec_type::structure::ECMemory) };
    let res = MEMORY_MAP.init(Mutex::new(RefCell::new(memory_map)));

    if res.is_err() {
        panic!("Failed to initialize MemoryMap");
    }

    init().await;

    loop {
        embassy_time::Timer::after_secs(10).await;

        let event = espi.wait_for_event().await;
        match event {
            Ok(espi::Event::Port0(port_event)) => {
                info!(
                    "eSPI Port 0, direction: {}, length: {}, offset: {}",
                    port_event.direction, port_event.length, port_event.offset,
                );
                espi.complete_port(0).await;
            }
            Ok(espi::Event::Port1(_)) => {
                info!("eSPI Port 1");
            }
            Ok(espi::Event::Port2(_port_event)) => {
                info!("eSPI Port 2");
            }
            Ok(espi::Event::Port3(_)) => {
                info!("eSPI Port 3");
            }
            Ok(espi::Event::Port4(_)) => {
                info!("eSPI Port 4");
            }
            Ok(espi::Event::Port80) => {
                info!("eSPI Port 80");
            }
            Ok(espi::Event::WireChange) => {
                info!("eSPI WireChange");
            }
            Err(_) => {
                error!("eSPI Failed");
            }
        }
    }
}
