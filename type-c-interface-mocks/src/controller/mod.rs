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
    /// Next results to return for [`type_c_interface::controller::pd::Pd::get_port_status`]
    pub next_result_get_port_status: VecDeque<Result<PortStatus, PdError>>,
    /// Next results to return for [`type_c_interface::controller::pd::Pd::clear_dead_battery_flag`]
    pub next_result_clear_dead_battery_flag: VecDeque<Result<(), PdError>>,
    /// Next results to return for [`type_c_interface::controller::pd::Pd::enable_sink_path`]
    pub next_result_enable_sink_path: VecDeque<Result<(), PdError>>,
    /// Next results to return for [`type_c_interface::controller::pd::Pd::get_pd_alert`]
    pub next_result_get_pd_alert: VecDeque<Result<Option<Ado>, PdError>>,
    /// Next results to return for [`type_c_interface::controller::pd::Pd::set_unconstrained_power`]
    pub next_result_set_unconstrained_power: VecDeque<Result<(), PdError>>,
    /// Next results to return for [`type_c_interface::controller::pd::Pd::get_other_vdm`]
    pub next_result_get_other_vdm: VecDeque<Result<OtherVdm, PdError>>,
    /// Next results to return for [`type_c_interface::controller::pd::Pd::get_attn_vdm`]
    pub next_result_get_attn_vdm: VecDeque<Result<AttnVdm, PdError>>,
    /// Next results to return for [`type_c_interface::controller::pd::Pd::send_vdm`]
    pub next_result_send_vdm: VecDeque<Result<(), PdError>>,
    /// Next results to return for [`type_c_interface::controller::pd::Pd::execute_drst`]
    pub next_result_execute_drst: VecDeque<Result<(), PdError>>,
    /// Next results to return for [`type_c_interface::controller::pd::Pd::get_dp_status`]
    pub next_result_get_dp_status: VecDeque<Result<DpStatus, PdError>>,
    /// Next results to return for [`type_c_interface::controller::pd::Pd::set_dp_config`]
    pub next_result_set_dp_config: VecDeque<Result<(), PdError>>,
    /// Next results to return for [`type_c_interface::controller::pd::Pd::set_tbt_config`]
    pub next_result_set_tbt_config: VecDeque<Result<(), PdError>>,
    /// Next results to return for [`type_c_interface::controller::pd::Pd::set_usb_control`]
    pub next_result_set_usb_control: VecDeque<Result<(), PdError>>,
    /// Next results to return for [`type_c_interface::ucsi::Lpm::execute_lpm_command`]
    pub next_result_execute_lpm_command: VecDeque<Result<Option<embedded_usb_pd::ucsi::lpm::ResponseData>, PdError>>,
    /// Next results to return for [`type_c_interface::controller::pd::Pd::hard_reset`]
    pub next_result_hard_reset: VecDeque<Result<(), PdError>>,
    /// Next results to return for [`type_c_interface::controller::pd::Pd::get_discovered_svids`]
    pub next_result_get_discovered_svids: VecDeque<Result<type_c_interface::control::svid::DiscoveredSvids, PdError>>,
    /// Next results to return for [`type_c_interface::controller::pd::Pd::get_discover_identity_sop_response`]
    pub next_result_get_discover_identity_sop_response:
        VecDeque<Result<embedded_usb_pd::vdm::structured::command::discover_identity::sop::ResponseVdos, PdError>>,
    /// Next results to return for [`type_c_interface::controller::pd::Pd::get_discover_identity_sop_prime_response`]
    pub next_result_get_discover_identity_sop_prime_response: VecDeque<
        Result<embedded_usb_pd::vdm::structured::command::discover_identity::sop_prime::ResponseVdos, PdError>,
    >,
}

impl Mock {
    /// Create a new mock with the given name
    pub fn new(name: &'static str) -> Self {
        Self {
            fn_calls: VecDeque::new(),
            name,
            next_result_get_port_status: VecDeque::new(),
            next_result_clear_dead_battery_flag: VecDeque::new(),
            next_result_enable_sink_path: VecDeque::new(),
            next_result_get_pd_alert: VecDeque::new(),
            next_result_set_unconstrained_power: VecDeque::new(),
            next_result_get_other_vdm: VecDeque::new(),
            next_result_get_attn_vdm: VecDeque::new(),
            next_result_send_vdm: VecDeque::new(),
            next_result_execute_drst: VecDeque::new(),
            next_result_get_dp_status: VecDeque::new(),
            next_result_set_dp_config: VecDeque::new(),
            next_result_set_tbt_config: VecDeque::new(),
            next_result_set_usb_control: VecDeque::new(),
            next_result_execute_lpm_command: VecDeque::new(),
            next_result_hard_reset: VecDeque::new(),
            next_result_get_discovered_svids: VecDeque::new(),
            next_result_get_discover_identity_sop_response: VecDeque::new(),
            next_result_get_discover_identity_sop_prime_response: VecDeque::new(),
        }
    }
}

impl Named for Mock {
    fn name(&self) -> &'static str {
        self.name
    }
}
