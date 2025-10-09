#[derive(Copy, Clone, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
/// ACPI Battery Methods
pub enum BatteryCmd {
    /// Battery Information eXtended
    GetBix = 1,
    /// Battery Status
    GetBst = 2,
    /// Power Source
    GetPsr = 3,
    /// Power source InFormation
    GetPif = 4,
    /// Battery Power State
    GetBps = 5,
    /// Battery Trip Point
    SetBtp = 6,
    /// Battery Power Threshold
    SetBpt = 7,
    /// Battery Power Characteristics
    GetBpc = 8,
    /// Battery Maintenance Control
    SetBmc = 9,
    /// Battery Maintenance Data
    GetBmd = 10,
    /// Battery Charge Time
    GetBct = 11,
    /// Battery Time
    GetBtm = 12,
    /// Battery Measurement Sampling Time
    SetBms = 13,
    /// Battery Measurement Averaging Interval
    SetBma = 14,
    /// Device Status
    GetSta = 15,
}
