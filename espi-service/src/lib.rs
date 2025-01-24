#![no_std]

pub mod espi_service;

#[derive(Copy, Clone, Debug)]
pub enum Message {
    // CAPS fields
    CapsFwVersion(u16),  // CAPS_FW_VERSION
    CapsSecureState(u8), // CAPS_SECURE_STATE
    CapsBootStatus(u8),  // CAPS_BOOT_STATUS
    CapsDebugMask(u16),  // CAPS_DEBUG_MASK
    CapsBatteryMask(u8), // CAPS_BATTERY_MASK
    CapsFanMask(u8),     // CAPS_FAN_MASK
    CapsTempMask(u8),    // CAPS_TEMP_MASK
    CapsHidMask(u8),     // CAPS_HID_MASK
    CapsKeyMask(u8),     // CAPS_KEY_MASK

    // BAT fields
    BatLastFullCharge(u32), // BAT_LAST_FULL_CHARGE (BIX)
    BatCycleCount(u32),     // BAT_CYCLE_COUNT (BIX)
    BatState(u32),          // BAT_STATE (BST)
    BatPresentRate(u32),    // BAT_PRESENT_RATE (BST)
    BatRemainCap(u32),      // BAT_REMAIN_CAP (BST)
    BatPresentVolt(u32),    // BAT_PRESENT_VOLT (BST)
    BatPsrState(u32),       // BAT_PSR_STATE (PSR/PIF)
    BatPsrMaxOut(u32),      // BAT_PSR_MAX_OUT (PIF)
    BatPsrMaxIn(u32),       // BAT_PSR_MAX_IN (PIF)
    BatPeakLevel(u32),      // BAT_PEAK_LEVEL (BPS)
    BatPeakPower(u32),      // BAT_PEAK_POWER (BPS/BPC)
    BatSusLevel(u32),       // BAT_SUS_LEVEL (BPS)
    BatSusPower(u32),       // BAT_SUS_POWER (BPS/PBC)
    BatPeakThres(u32),      // BAT_PEAK_THRES (BPT)
    BatSusThres(u32),       // BAT_SUS_THRES (BPT)
    BatTripThres(u32),      // BAT_TRIP_THRES (BTP)
    BatBmcData(u32),        // BAT_BMC_DATA (BMC)
    BatBmdStatus(u32),      // BAT_BMD_STATUS (BMD)
    BatBmdFlags(u32),       // BAT_BMD_FLAGS (BMD)
    BatBmdCount(u32),       // BAT_BMD_COUNT (BMD)
    BatChargeTime(u32),     // BAT_CHARGE_TIME (BCT)
    BatRunTime(u32),        // BAT_RUN_TIME (BTM)
    BatSampleTime(u32),     // BAT_SAMPLE_TIME (BMS/BMA)

    // MPTF fields
    MptfTmp1Val(u32),     // THM_TMP1_VAL (TMP)
    MptfTmp1Timeout(u32), // THM_TMP1_TIMEOUT (EC_THM_SET/GET_THRS)
    MptfTmp1Low(u32),     // THM_TMP1_LOW (EC_THM_SET/GET_THRS)
    MptfTmp1High(u32),    // THM_TMP1_HIGH (EC_THM_SET/GET_THRS)
    MptfCoolMode(u32),    // THM_COOL_MODE (EC_THM_SET_SCP)
    MptfFanOnTemp(u32),   // THM_FAN_ON_TEMP (GET/SET VAR)
    MptfFanRampTemp(u32), // THM_FAN_RAMP_TEMP (GET/SET VAR)
    MptfFanMaxTemp(u32),  // THM_FAN_MAX_TEMP (GET/SET VAR)
    MptfFanCrtTemp(u32),  // THM_FAN_CRT_TEMP (GET/SET VAR)
    MptfFanHotTemp(u32),  // THM_FAN_HOT_TEMP (GET/SET VAR PROCHOT notification)
    MptfFanMaxRpm(u32),   // THM_FAN_MAX_RPM (GET/SET VAR)
    MptfFanRpm(u32),      // THM_FAN_RPM (GET VAR)
    MptfDbaLimit(u32),    // THM_DBA_LIMIT (GET/SET VAR)
    MptfSonLimit(u32),    // THM_SON_LIMIT (GET/SET VAR)
    MptfMaLimit(u32),     // THM_MA_LIMIT (GET/SET VAR)

    // RTC fields
    RtcCapability(u32),  // TAS_CAPABILITY (GCP)
    RtcYear(u16),        // TAS_YEAR (GRT/SRT)
    RtcMonth(u8),        // TAS_MONTH (GRT/SRT)
    RtcDay(u8),          // TAS_DAY (GRT/SRT)
    RtcHour(u8),         // TAS_HOUR (GRT/SRT)
    RtcMinute(u8),       // TAS_MINUTE (GRT/SRT)
    RtcSecond(u8),       // TAS_SECOND (GRT/SRT)
    RtcValid(u8),        // TAS_VALID (GRT/SRT)
    RtcMs(u16),          // TAS_MS (GRT/SRT)
    RtcTimeZone(u16),    // TAS_TIME_ZONE (GRT/SRT)
    RtcDaylight(u8),     // TAS_DAYLIGHT (GRT/SRT)
    RtcAlarmStatus(u32), // TAS_ALARM_STATUS (GWS/CWS)
    RtcAcTimeVal(u32),   // TAS_AC_TIME_VAL (STV/TIV)
    RtcDcTimeVal(u32),   // TAS_DC_TIME_VAL (STV/TIV)
}
