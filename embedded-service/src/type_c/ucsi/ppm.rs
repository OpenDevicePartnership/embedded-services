use bitfield::bitfield;

use crate::type_c::Error;

bitfield! {
    /// PPM notifications that can be enabled, see spec for more details
    #[derive(Copy, Clone)]
    #[cfg_attr(feature = "defmt", derive(defmt::Format))]
    pub struct SetNotificationEnableData(u32);
    impl Debug;
    pub cmd_complete, set_cmd_complete: 0, 0;
    pub external_supply_change, set_external_supply_change: 1, 1;
    pub power_op_mode_change, set_power_op_mode_change: 2, 2;
    pub attention, set_attention: 3, 3;
    pub fw_update_req, set_fw_update_req: 4, 4;
    pub provider_caps_change, set_provider_caps_change: 5, 5;
    pub power_lvl_change, set_power_lvl_change: 6, 6;
    pub pd_reset_complete, set_pd_reset_complete: 7, 7;
    pub cam_change, set_cam_change: 8, 8;
    pub battery_charge_change, set_battery_charge_change: 9, 9;
    pub security_req, set_security_req: 10, 10;
    pub connector_partner_change, set_connector_partner_change: 11, 11;
    pub power_dir_change, set_power_dir_change: 12, 12;
    pub set_retimer_mode, set_set_retimer_mode: 13, 13;
    pub connect_change, set_connect_change: 14, 14;
    pub error, set_error: 15, 15;
    pub sink_path_change, set_sink_path_change: 16, 16;
}

/// Commands that only affect the PPM level and don't need to be sent to an LPM
#[derive(Copy, Clone, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Command {
    Reset,
    Cancel,
    AckCcCi,
    SetNotificationEnable(SetNotificationEnableData),
}

/// PPM command response data
#[derive(Copy, Clone, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum ResponseData {
    Complete,
}

impl Into<Result<ResponseData, Error>> for ResponseData {
    fn into(self) -> Result<ResponseData, Error> {
        Ok(self)
    }
}
