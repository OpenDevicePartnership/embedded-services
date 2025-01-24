use core::sync::atomic::{AtomicU16, AtomicU32, AtomicU8, Ordering};

use embassy_sync::once_lock::OnceLock;
use embedded_services::{
    comms::{self, EndpointID, External},
    info,
};

#[repr(C)]
#[derive(Default)]
struct MemoryMap {
    // CAPS fields
    fw_version: AtomicU16,  // CAPS_FW_VERSION
    secure_state: AtomicU8, // CAPS_SECURE_STATE
    boot_status: AtomicU8,  // CAPS_BOOT_STATUS
    debug_mask: AtomicU16,  // CAPS_DEBUG_MASK
    battery_mask: AtomicU8, // CAPS_BATTERY_MASK
    fan_mask: AtomicU8,     // CAPS_FAN_MASK
    temp_mask: AtomicU8,    // CAPS_TEMP_MASK
    hid_mask: AtomicU8,     // CAPS_HID_MASK
    key_mask: AtomicU8,     // CAPS_KEY_MASK

    // BAT fields
    last_full_charge: AtomicU32, // BAT_LAST_FULL_CHARGE (BIX)
    cycle_count: AtomicU32,      // BAT_CYCLE_COUNT (BIX)
    state: AtomicU32,            // BAT_STATE (BST)
    present_rate: AtomicU32,     // BAT_PRESENT_RATE (BST)
    remain_cap: AtomicU32,       // BAT_REMAIN_CAP (BST)
    present_volt: AtomicU32,     // BAT_PRESENT_VOLT (BST)
    psr_state: AtomicU32,        // BAT_PSR_STATE (PSR/PIF)
    psr_max_out: AtomicU32,      // BAT_PSR_MAX_OUT (PIF)
    psr_max_in: AtomicU32,       // BAT_PSR_MAX_IN (PIF)
    peak_level: AtomicU32,       // BAT_PEAK_LEVEL (BPS)
    peak_power: AtomicU32,       // BAT_PEAK_POWER (BPS/BPC)
    sus_level: AtomicU32,        // BAT_SUS_LEVEL (BPS)
    sus_power: AtomicU32,        // BAT_SUS_POWER (BPS/PBC)
    peak_thres: AtomicU32,       // BAT_PEAK_THRES (BPT)
    sus_thres: AtomicU32,        // BAT_SUS_THRES (BPT)
    trip_thres: AtomicU32,       // BAT_TRIP_THRES (BTP)
    bmc_data: AtomicU32,         // BAT_BMC_DATA (BMC)
    bmd_status: AtomicU32,       // BAT_BMD_STATUS (BMD)
    bmd_flags: AtomicU32,        // BAT_BMD_FLAGS (BMD)
    bmd_count: AtomicU32,        // BAT_BMD_COUNT (BMD)
    charge_time: AtomicU32,      // BAT_CHARGE_TIME (BCT)
    run_time: AtomicU32,         // BAT_RUN_TIME (BTM)
    sample_time: AtomicU32,      // BAT_SAMPLE_TIME (BMS/BMA)

    // MPTF fields
    tmp1_val: AtomicU32,      // THM_TMP1_VAL (TMP)
    tmp1_timeout: AtomicU32,  // THM_TMP1_TIMEOUT (EC_THM_SET/GET_THRS)
    tmp1_low: AtomicU32,      // THM_TMP1_LOW (EC_THM_SET/GET_THRS)
    tmp1_high: AtomicU32,     // THM_TMP1_HIGH (EC_THM_SET/GET_THRS)
    cool_mode: AtomicU32,     // THM_COOL_MODE (EC_THM_SET_SCP)
    fan_on_temp: AtomicU32,   // THM_FAN_ON_TEMP (GET/SET VAR)
    fan_ramp_temp: AtomicU32, // THM_FAN_RAMP_TEMP (GET/SET VAR)
    fan_max_temp: AtomicU32,  // THM_FAN_MAX_TEMP (GET/SET VAR)
    fan_crt_temp: AtomicU32,  // THM_FAN_CRT_TEMP (GET/SET VAR)
    fan_hot_temp: AtomicU32,  // THM_FAN_HOT_TEMP (GET/SET VAR PROCHOT notification)
    fan_max_rpm: AtomicU32,   // THM_FAN_MAX_RPM (GET/SET VAR)
    fan_rpm: AtomicU32,       // THM_FAN_RPM (GET VAR)
    dba_limit: AtomicU32,     // THM_DBA_LIMIT (GET/SET VAR)
    son_limit: AtomicU32,     // THM_SON_LIMIT (GET/SET VAR)
    ma_limit: AtomicU32,      // THM_MA_LIMIT (GET/SET VAR)

    // RTC fields
    capability: AtomicU32,   // TAS_CAPABILITY (GCP)
    year: AtomicU16,         // TAS_YEAR (GRT/SRT)
    month: AtomicU8,         // TAS_MONTH (GRT/SRT)
    day: AtomicU8,           // TAS_DAY (GRT/SRT)
    hour: AtomicU8,          // TAS_HOUR (GRT/SRT)
    minute: AtomicU8,        // TAS_MINUTE (GRT/SRT)
    second: AtomicU8,        // TAS_SECOND (GRT/SRT)
    valid: AtomicU8,         // TAS_VALID (GRT/SRT)
    ms: AtomicU16,           // TAS_MS (GRT/SRT)
    time_zone: AtomicU16,    // TAS_TIME_ZONE (GRT/SRT)
    daylight: AtomicU8,      // TAS_DAYLIGHT (GRT/SRT)
    alarm_status: AtomicU32, // TAS_ALARM_STATUS (GWS/CWS)
    ac_time_val: AtomicU32,  // TAS_AC_TIME_VAL (STV/TIV)
    dc_time_val: AtomicU32,  // TAS_DC_TIME_VAL (STV/TIV)
}

pub struct Service {
    pub endpoint: comms::Endpoint,
    memory_map: MemoryMap,
    // This is can be an Embassy signal or channel or whatever Embassy async notification construct
    //signal: Signal<NoopRawMutex, TxMessage>,
}

impl Service {
    pub fn new() -> Self {
        Service {
            endpoint: comms::Endpoint::uninit(EndpointID::External(External::Host)),
            memory_map: MemoryMap::default(),
            //signal: Signal::new(),
        }
    }
}

impl comms::MailboxDelegate for Service {
    fn receive(&self, message: &comms::Message) {
        if let Some(msg) = message.data.get::<super::Message>() {
            info!("Receive message to send to the host");
            update_memory_map(msg);
        }
    }
}

static ESPI_SERVICE: OnceLock<Service> = OnceLock::new();

// Initialize eSPI service and register it with the transport service
pub async fn init() {
    let espi_service = ESPI_SERVICE.get_or_init(|| Service::new());

    comms::register_endpoint(espi_service, &espi_service.endpoint)
        .await
        .unwrap();
}

enum Unsigned {
    U8(u8),
    U16(u16),
    U32(u32),
}

impl From<Unsigned> for u8 {
    fn from(value: Unsigned) -> Self {
        match value {
            Unsigned::U8(v) => v,
            _ => panic!("Invalid conversion"),
        }
    }
}

impl From<Unsigned> for u16 {
    fn from(value: Unsigned) -> Self {
        match value {
            Unsigned::U16(v) => v,
            _ => panic!("Invalid conversion"),
        }
    }
}

impl From<Unsigned> for u32 {
    fn from(value: Unsigned) -> Self {
        match value {
            Unsigned::U32(v) => v,
            _ => panic!("Invalid conversion"),
        }
    }
}

fn offset_to_message(offset: usize, value: Unsigned) -> super::Message {
    match offset {
        0 => super::Message::CapsFwVersion(value.into()),
        2 => super::Message::CapsSecureState(value.into()),
        3 => super::Message::CapsBootStatus(value.into()),
        4 => super::Message::CapsDebugMask(value.into()),
        6 => super::Message::CapsBatteryMask(value.into()),
        7 => super::Message::CapsFanMask(value.into()),
        8 => super::Message::CapsTempMask(value.into()),
        9 => super::Message::CapsHidMask(value.into()),
        10 => super::Message::CapsKeyMask(value.into()),

        16 => super::Message::BatLastFullCharge(value.into()),
        20 => super::Message::BatCycleCount(value.into()),
        24 => super::Message::BatState(value.into()),
        28 => super::Message::BatPresentRate(value.into()),
        32 => super::Message::BatRemainCap(value.into()),
        36 => super::Message::BatPresentVolt(value.into()),
        40 => super::Message::BatPsrState(value.into()),
        44 => super::Message::BatPsrMaxOut(value.into()),
        48 => super::Message::BatPsrMaxIn(value.into()),
        52 => super::Message::BatPeakLevel(value.into()),
        56 => super::Message::BatPeakPower(value.into()),
        60 => super::Message::BatSusLevel(value.into()),
        64 => super::Message::BatSusPower(value.into()),
        68 => super::Message::BatPeakThres(value.into()),
        72 => super::Message::BatSusThres(value.into()),
        76 => super::Message::BatTripThres(value.into()),
        80 => super::Message::BatBmcData(value.into()),
        84 => super::Message::BatBmdStatus(value.into()),
        88 => super::Message::BatBmdFlags(value.into()),
        92 => super::Message::BatBmdCount(value.into()),
        96 => super::Message::BatChargeTime(value.into()),
        100 => super::Message::BatRunTime(value.into()),
        104 => super::Message::BatSampleTime(value.into()),

        112 => super::Message::MptfTmp1Val(value.into()),
        116 => super::Message::MptfTmp1Timeout(value.into()),
        120 => super::Message::MptfTmp1Low(value.into()),
        124 => super::Message::MptfTmp1High(value.into()),
        128 => super::Message::MptfCoolMode(value.into()),
        132 => super::Message::MptfFanOnTemp(value.into()),
        136 => super::Message::MptfFanRampTemp(value.into()),
        140 => super::Message::MptfFanMaxTemp(value.into()),
        144 => super::Message::MptfFanCrtTemp(value.into()),
        148 => super::Message::MptfFanHotTemp(value.into()),
        152 => super::Message::MptfFanMaxRpm(value.into()),
        156 => super::Message::MptfFanRpm(value.into()),
        160 => super::Message::MptfDbaLimit(value.into()),
        164 => super::Message::MptfSonLimit(value.into()),
        168 => super::Message::MptfMaLimit(value.into()),

        176 => super::Message::RtcCapability(value.into()),
        180 => super::Message::RtcYear(value.into()),
        182 => super::Message::RtcMonth(value.into()),
        183 => super::Message::RtcDay(value.into()),
        184 => super::Message::RtcHour(value.into()),
        185 => super::Message::RtcMinute(value.into()),
        186 => super::Message::RtcSecond(value.into()),
        187 => super::Message::RtcValid(value.into()),
        188 => super::Message::RtcMs(value.into()),
        190 => super::Message::RtcTimeZone(value.into()),
        192 => super::Message::RtcDaylight(value.into()),
        193 => super::Message::RtcAlarmStatus(value.into()),
        197 => super::Message::RtcAcTimeVal(value.into()),
        201 => super::Message::RtcDcTimeVal(value.into()),
        _ => panic!("Invalid offset"),
    }
}

fn update_memory_map(msg: &super::Message) {
    let memory_map = &ESPI_SERVICE.get_or_init(|| Service::new()).memory_map;
    match msg {
        super::Message::CapsFwVersion(fw_version) => memory_map.fw_version.store(*fw_version, Ordering::Relaxed),
        super::Message::CapsSecureState(secure_state) => {
            memory_map.secure_state.store(*secure_state, Ordering::Relaxed)
        }
        super::Message::CapsBootStatus(boot_status) => memory_map.boot_status.store(*boot_status, Ordering::Relaxed),
        super::Message::CapsDebugMask(debug_mask) => memory_map.debug_mask.store(*debug_mask, Ordering::Relaxed),
        super::Message::CapsBatteryMask(battery_mask) => {
            memory_map.battery_mask.store(*battery_mask, Ordering::Relaxed)
        }
        super::Message::CapsFanMask(fan_mask) => memory_map.fan_mask.store(*fan_mask, Ordering::Relaxed),
        super::Message::CapsTempMask(temp_mask) => memory_map.temp_mask.store(*temp_mask, Ordering::Relaxed),
        super::Message::CapsHidMask(hid_mask) => memory_map.hid_mask.store(*hid_mask, Ordering::Relaxed),
        super::Message::CapsKeyMask(key_mask) => memory_map.key_mask.store(*key_mask, Ordering::Relaxed),

        super::Message::BatLastFullCharge(last_full_charge) => {
            memory_map.last_full_charge.store(*last_full_charge, Ordering::Relaxed)
        }
        super::Message::BatCycleCount(cycle_count) => memory_map.cycle_count.store(*cycle_count, Ordering::Relaxed),
        super::Message::BatState(state) => memory_map.state.store(*state, Ordering::Relaxed),
        super::Message::BatPresentRate(present_rate) => memory_map.present_rate.store(*present_rate, Ordering::Relaxed),
        super::Message::BatRemainCap(remain_cap) => {
            memory_map.remain_cap.store(*remain_cap, Ordering::Relaxed);
        }
        super::Message::BatPresentVolt(present_volt) => memory_map.present_volt.store(*present_volt, Ordering::Relaxed),
        super::Message::BatPsrState(psr_state) => memory_map.psr_state.store(*psr_state, Ordering::Relaxed),
        super::Message::BatPsrMaxOut(psr_max_out) => memory_map.psr_max_out.store(*psr_max_out, Ordering::Relaxed),
        super::Message::BatPsrMaxIn(psr_max_in) => memory_map.psr_max_in.store(*psr_max_in, Ordering::Relaxed),
        super::Message::BatPeakLevel(peak_level) => memory_map.peak_level.store(*peak_level, Ordering::Relaxed),
        super::Message::BatPeakPower(peak_power) => memory_map.peak_power.store(*peak_power, Ordering::Relaxed),
        super::Message::BatSusLevel(sus_level) => memory_map.sus_level.store(*sus_level, Ordering::Relaxed),
        super::Message::BatSusPower(sus_power) => memory_map.sus_power.store(*sus_power, Ordering::Relaxed),
        super::Message::BatPeakThres(peak_thres) => memory_map.peak_thres.store(*peak_thres, Ordering::Relaxed),
        super::Message::BatSusThres(sus_thres) => memory_map.sus_thres.store(*sus_thres, Ordering::Relaxed),
        super::Message::BatTripThres(trip_thres) => memory_map.trip_thres.store(*trip_thres, Ordering::Relaxed),
        super::Message::BatBmcData(bmc_data) => memory_map.bmc_data.store(*bmc_data, Ordering::Relaxed),
        super::Message::BatBmdStatus(bmd_status) => memory_map.bmd_status.store(*bmd_status, Ordering::Relaxed),
        super::Message::BatBmdFlags(bmd_flags) => memory_map.bmd_flags.store(*bmd_flags, Ordering::Relaxed),
        super::Message::BatBmdCount(bmd_count) => memory_map.bmd_count.store(*bmd_count, Ordering::Relaxed),
        super::Message::BatChargeTime(charge_time) => memory_map.charge_time.store(*charge_time, Ordering::Relaxed),
        super::Message::BatRunTime(run_time) => memory_map.run_time.store(*run_time, Ordering::Relaxed),
        super::Message::BatSampleTime(sample_time) => memory_map.sample_time.store(*sample_time, Ordering::Relaxed),

        super::Message::MptfTmp1Val(tmp1_val) => memory_map.tmp1_val.store(*tmp1_val, Ordering::Relaxed),
        super::Message::MptfTmp1Timeout(tmp1_timeout) => {
            memory_map.tmp1_timeout.store(*tmp1_timeout, Ordering::Relaxed)
        }
        super::Message::MptfTmp1Low(tmp1_low) => memory_map.tmp1_low.store(*tmp1_low, Ordering::Relaxed),
        super::Message::MptfTmp1High(tmp1_high) => memory_map.tmp1_high.store(*tmp1_high, Ordering::Relaxed),
        super::Message::MptfCoolMode(cool_mode) => memory_map.cool_mode.store(*cool_mode, Ordering::Relaxed),
        super::Message::MptfFanOnTemp(fan_on_temp) => memory_map.fan_on_temp.store(*fan_on_temp, Ordering::Relaxed),
        super::Message::MptfFanRampTemp(fan_ramp_temp) => {
            memory_map.fan_ramp_temp.store(*fan_ramp_temp, Ordering::Relaxed)
        }
        super::Message::MptfFanMaxTemp(fan_max_temp) => memory_map.fan_max_temp.store(*fan_max_temp, Ordering::Relaxed),
        super::Message::MptfFanCrtTemp(fan_crt_temp) => memory_map.fan_crt_temp.store(*fan_crt_temp, Ordering::Relaxed),
        super::Message::MptfFanHotTemp(fan_hot_temp) => memory_map.fan_hot_temp.store(*fan_hot_temp, Ordering::Relaxed),
        super::Message::MptfFanMaxRpm(fan_max_rpm) => memory_map.fan_max_rpm.store(*fan_max_rpm, Ordering::Relaxed),
        super::Message::MptfFanRpm(fan_rpm) => memory_map.fan_rpm.store(*fan_rpm, Ordering::Relaxed),
        super::Message::MptfDbaLimit(dba_limit) => memory_map.dba_limit.store(*dba_limit, Ordering::Relaxed),
        super::Message::MptfSonLimit(son_limit) => memory_map.son_limit.store(*son_limit, Ordering::Relaxed),
        super::Message::MptfMaLimit(ma_limit) => memory_map.ma_limit.store(*ma_limit, Ordering::Relaxed),

        super::Message::RtcCapability(capability) => memory_map.capability.store(*capability, Ordering::Relaxed),
        super::Message::RtcYear(year) => memory_map.year.store(*year, Ordering::Relaxed),
        super::Message::RtcMonth(month) => memory_map.month.store(*month, Ordering::Relaxed),
        super::Message::RtcDay(day) => memory_map.day.store(*day, Ordering::Relaxed),
        super::Message::RtcHour(hour) => memory_map.hour.store(*hour, Ordering::Relaxed),
        super::Message::RtcMinute(minute) => memory_map.minute.store(*minute, Ordering::Relaxed),
        super::Message::RtcSecond(second) => memory_map.second.store(*second, Ordering::Relaxed),
        super::Message::RtcValid(valid) => memory_map.valid.store(*valid, Ordering::Relaxed),
        super::Message::RtcMs(ms) => memory_map.ms.store(*ms, Ordering::Relaxed),
        super::Message::RtcTimeZone(time_zone) => memory_map.time_zone.store(*time_zone, Ordering::Relaxed),
        super::Message::RtcDaylight(daylight) => memory_map.daylight.store(*daylight, Ordering::Relaxed),
        super::Message::RtcAlarmStatus(alarm_status) => memory_map.alarm_status.store(*alarm_status, Ordering::Relaxed),
        super::Message::RtcAcTimeVal(ac_time_val) => memory_map.ac_time_val.store(*ac_time_val, Ordering::Relaxed),
        super::Message::RtcDcTimeVal(dc_time_val) => memory_map.dc_time_val.store(*dc_time_val, Ordering::Relaxed),
    }
}
