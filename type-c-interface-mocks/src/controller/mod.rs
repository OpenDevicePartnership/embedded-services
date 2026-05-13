//! Mock controller implementations for testing

use std::collections::VecDeque;

use embedded_services::named::Named;
use embedded_usb_pd::{PdError, ado::Ado};
use type_c_interface::control::{
    dp::DpStatus,
    pd::PortStatus,
    vdm::{AttnVdm, OtherVdm},
};

pub mod pd;
pub mod ucsi;

/// Contains a controller function call and its arguments
pub enum FnCall {
    Pd(pd::FnCall),
    Ucsi(ucsi::FnCall),
}

/// Mock PD controller for use in tests
pub struct Mock {
    name: &'static str,
    /// Recorded function calls
    pub fn_calls: VecDeque<FnCall>,
    /// Next result to return for [`type_c_interface::controller::pd::Pd::get_port_status`]
    pub next_result_get_port_status: Option<Result<PortStatus, PdError>>,
    /// Next result to return for [`type_c_interface::controller::pd::Pd::clear_dead_battery_flag`]
    pub next_result_clear_dead_battery_flag: Option<Result<(), PdError>>,
    /// Next result to return for [`type_c_interface::controller::pd::Pd::enable_sink_path`]
    pub next_result_enable_sink_path: Option<Result<(), PdError>>,
    /// Next result to return for [`type_c_interface::controller::pd::Pd::get_pd_alert`]
    pub next_result_get_pd_alert: Option<Result<Option<Ado>, PdError>>,
    /// Next result to return for [`type_c_interface::controller::pd::Pd::set_unconstrained_power`]
    pub next_result_set_unconstrained_power: Option<Result<(), PdError>>,
    /// Next result to return for [`type_c_interface::controller::pd::Pd::get_other_vdm`]
    pub next_result_get_other_vdm: Option<Result<OtherVdm, PdError>>,
    /// Next result to return for [`type_c_interface::controller::pd::Pd::get_attn_vdm`]
    pub next_result_get_attn_vdm: Option<Result<AttnVdm, PdError>>,
    /// Next result to return for [`type_c_interface::controller::pd::Pd::send_vdm`]
    pub next_result_send_vdm: Option<Result<(), PdError>>,
    /// Next result to return for [`type_c_interface::controller::pd::Pd::execute_drst`]
    pub next_result_execute_drst: Option<Result<(), PdError>>,
    /// Next result to return for [`type_c_interface::controller::pd::Pd::get_dp_status`]
    pub next_result_get_dp_status: Option<Result<DpStatus, PdError>>,
    /// Next result to return for [`type_c_interface::controller::pd::Pd::set_dp_config`]
    pub next_result_set_dp_config: Option<Result<(), PdError>>,
    /// Next result to return for [`type_c_interface::controller::pd::Pd::set_tbt_config`]
    pub next_result_set_tbt_config: Option<Result<(), PdError>>,
    /// Next result to return for [`type_c_interface::controller::pd::Pd::set_usb_control`]
    pub next_result_set_usb_control: Option<Result<(), PdError>>,
    /// Next result to return for [`type_c_interface::ucsi::Lpm::execute_lpm_command`]
    pub next_result_execute_lpm_command: Option<Result<Option<embedded_usb_pd::ucsi::lpm::ResponseData>, PdError>>,
}

impl Mock {
    /// Create a new mock with the given name
    pub fn new(name: &'static str) -> Self {
        Self {
            fn_calls: VecDeque::new(),
            name,
            next_result_get_port_status: None,
            next_result_clear_dead_battery_flag: None,
            next_result_enable_sink_path: None,
            next_result_get_pd_alert: None,
            next_result_set_unconstrained_power: None,
            next_result_get_other_vdm: None,
            next_result_get_attn_vdm: None,
            next_result_send_vdm: None,
            next_result_execute_drst: None,
            next_result_get_dp_status: None,
            next_result_set_dp_config: None,
            next_result_set_tbt_config: None,
            next_result_set_usb_control: None,
            next_result_execute_lpm_command: None,
        }
    }
}

impl Named for Mock {
    fn name(&self) -> &'static str {
        self.name
    }
}
