/// Standard MPTF requests expected by the thermal subsystem
#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum ThermalCmd {
    /// EC_THM_GET_TMP = 0x1
    GetTmp = 1,
    /// EC_THM_SET_THRS = 0x2
    SetThrs = 2,
    /// EC_THM_GET_THRS = 0x3
    GetThrs = 3,
    /// EC_THM_SET_SCP = 0x4
    SetScp = 4,
    /// EC_THM_GET_VAR = 0x5
    GetVar = 5,
    /// EC_THM_SET_VAR = 0x6
    SetVar = 6,
}
