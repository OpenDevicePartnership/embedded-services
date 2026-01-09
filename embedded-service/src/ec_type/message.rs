//! EC Internal Messages

#[allow(missing_docs)]
#[derive(Clone, Copy, Debug)]
pub enum CapabilitiesMessage {
    Events(u32),
    FwVersion(super::structure::Version),
    SecureState(u8),
    BootStatus(u8),
    FanMask(u8),
    BatteryMask(u8),
    TempMask(u16),
    KeyMask(u16),
    DebugMask(u16),
}

#[allow(missing_docs)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TimeAlarmMessage {
    Events(u32),
    Capability(u32),
    Year(u16),
    Month(u8),
    Day(u8),
    Hour(u8),
    Minute(u8),
    Second(u8),
    Valid(u8),
    Daylight(u8),
    Res1(u8),
    Milli(u16),
    TimeZone(u16),
    Res2(u16),
    AlarmStatus(u32),
    AcTimeVal(u32),
    DcTimeVal(u32),
}

#[allow(missing_docs)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BatteryMessage {
    Events(u32),
    Status(u32),
    LastFullCharge(u32),
    CycleCount(u32),
    State(u32),
    PresentRate(u32),
    RemainCap(u32),
    PresentVolt(u32),
    PsrState(u32),
    PsrMaxOut(u32),
    PsrMaxIn(u32),
    PeakLevel(u32),
    PeakPower(u32),
    SusLevel(u32),
    SusPower(u32),
    PeakThres(u32),
    SusThres(u32),
    TripThres(u32),
    BmcData(u32),
    BmdData(u32),
    BmdFlags(u32),
    BmdCount(u32),
    ChargeTime(u32),
    RunTime(u32),
    SampleTime(u32),
}

#[allow(missing_docs)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ThermalMessage {
    Events(u32),
    CoolMode(u32),
    DbaLimit(u32),
    SonneLimit(u32),
    MaLimit(u32),
    Fan1OnTemp(u32),
    Fan1RampTemp(u32),
    Fan1MaxTemp(u32),
    Fan1CrtTemp(u32),
    Fan1HotTemp(u32),
    Fan1MaxRpm(u32),
    Fan1CurRpm(u32),
    Tmp1Val(u32),
    Tmp1Timeout(u32),
    Tmp1Low(u32),
    Tmp1High(u32),
}
