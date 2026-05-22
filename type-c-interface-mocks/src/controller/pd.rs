//! Mock implementation of [`type_c_interface::controller::pd::Pd`]

use embedded_usb_pd::{LocalPortId, PdError, ado::Ado};
use type_c_interface::{
    control::{
        dp::{DpConfig, DpStatus},
        pd::PortStatus,
        tbt::TbtConfig,
        usb::UsbControlConfig,
        vdm::{AttnVdm, OtherVdm, SendVdm},
    },
    controller::pd::Pd,
};

use super::FnCall as ControllerFnCall;
use super::Mock;

/// Contains a [`Pd`] function call and its arguments
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FnCall {
    GetPortStatus(LocalPortId),
    ClearDeadBatteryFlag(LocalPortId),
    EnableSinkPath(LocalPortId, bool),
    GetPdAlert(LocalPortId),
    SetUnconstrainedPower(LocalPortId, bool),
    GetOtherVdm(LocalPortId),
    GetAttnVdm(LocalPortId),
    SendVdm(LocalPortId, SendVdm),
    ExecuteDrst(LocalPortId),
    GetDpStatus(LocalPortId),
    SetDpConfig(LocalPortId, DpConfig),
    SetTbtConfig(LocalPortId, TbtConfig),
    SetUsbControl(LocalPortId, UsbControlConfig),
    HardReset(LocalPortId),
    GetDiscoveredSvids(LocalPortId),
    GetDiscoverIdentitySopResponse(LocalPortId),
    GetDiscoverIdentitySopPrimeResponse(LocalPortId),
}

impl Pd for Mock {
    async fn get_port_status(&mut self, port: LocalPortId) -> Result<PortStatus, PdError> {
        self.fn_calls
            .push_back(ControllerFnCall::Pd(FnCall::GetPortStatus(port)));
        self.next_result_get_port_status
            .pop_front()
            .expect("next_result_get_port_status not set")
    }

    async fn clear_dead_battery_flag(&mut self, port: LocalPortId) -> Result<(), PdError> {
        self.fn_calls
            .push_back(ControllerFnCall::Pd(FnCall::ClearDeadBatteryFlag(port)));
        self.next_result_clear_dead_battery_flag
            .pop_front()
            .expect("next_result_clear_dead_battery_flag not set")
    }

    async fn enable_sink_path(&mut self, port: LocalPortId, enable: bool) -> Result<(), PdError> {
        self.fn_calls
            .push_back(ControllerFnCall::Pd(FnCall::EnableSinkPath(port, enable)));
        self.next_result_enable_sink_path
            .pop_front()
            .expect("next_result_enable_sink_path not set")
    }

    async fn get_pd_alert(&mut self, port: LocalPortId) -> Result<Option<Ado>, PdError> {
        self.fn_calls.push_back(ControllerFnCall::Pd(FnCall::GetPdAlert(port)));
        self.next_result_get_pd_alert
            .pop_front()
            .expect("next_result_get_pd_alert not set")
    }

    async fn set_unconstrained_power(&mut self, port: LocalPortId, unconstrained: bool) -> Result<(), PdError> {
        self.fn_calls
            .push_back(ControllerFnCall::Pd(FnCall::SetUnconstrainedPower(port, unconstrained)));
        self.next_result_set_unconstrained_power
            .pop_front()
            .expect("next_result_set_unconstrained_power not set")
    }

    async fn get_other_vdm(&mut self, port: LocalPortId) -> Result<OtherVdm, PdError> {
        self.fn_calls.push_back(ControllerFnCall::Pd(FnCall::GetOtherVdm(port)));
        self.next_result_get_other_vdm
            .pop_front()
            .expect("next_result_get_other_vdm not set")
    }

    async fn get_attn_vdm(&mut self, port: LocalPortId) -> Result<AttnVdm, PdError> {
        self.fn_calls.push_back(ControllerFnCall::Pd(FnCall::GetAttnVdm(port)));
        self.next_result_get_attn_vdm
            .pop_front()
            .expect("next_result_get_attn_vdm not set")
    }

    async fn send_vdm(&mut self, port: LocalPortId, tx_vdm: SendVdm) -> Result<(), PdError> {
        self.fn_calls
            .push_back(ControllerFnCall::Pd(FnCall::SendVdm(port, tx_vdm)));
        self.next_result_send_vdm
            .pop_front()
            .expect("next_result_send_vdm not set")
    }

    async fn execute_drst(&mut self, port: LocalPortId) -> Result<(), PdError> {
        self.fn_calls.push_back(ControllerFnCall::Pd(FnCall::ExecuteDrst(port)));
        self.next_result_execute_drst
            .pop_front()
            .expect("next_result_execute_drst not set")
    }

    async fn get_dp_status(&mut self, port: LocalPortId) -> Result<DpStatus, PdError> {
        self.fn_calls.push_back(ControllerFnCall::Pd(FnCall::GetDpStatus(port)));
        self.next_result_get_dp_status
            .pop_front()
            .expect("next_result_get_dp_status not set")
    }

    async fn set_dp_config(&mut self, port: LocalPortId, config: DpConfig) -> Result<(), PdError> {
        self.fn_calls
            .push_back(ControllerFnCall::Pd(FnCall::SetDpConfig(port, config)));
        self.next_result_set_dp_config
            .pop_front()
            .expect("next_result_set_dp_config not set")
    }

    async fn set_tbt_config(&mut self, port: LocalPortId, config: TbtConfig) -> Result<(), PdError> {
        self.fn_calls
            .push_back(ControllerFnCall::Pd(FnCall::SetTbtConfig(port, config)));
        self.next_result_set_tbt_config
            .pop_front()
            .expect("next_result_set_tbt_config not set")
    }

    async fn set_usb_control(&mut self, port: LocalPortId, config: UsbControlConfig) -> Result<(), PdError> {
        self.fn_calls
            .push_back(ControllerFnCall::Pd(FnCall::SetUsbControl(port, config)));
        self.next_result_set_usb_control
            .pop_front()
            .expect("next_result_set_usb_control not set")
    }

    async fn hard_reset(&mut self, port: LocalPortId) -> Result<(), PdError> {
        self.fn_calls.push_back(ControllerFnCall::Pd(FnCall::HardReset(port)));
        self.next_result_hard_reset
            .pop_front()
            .expect("next_result_hard_reset not set")
    }

    async fn get_discovered_svids(
        &mut self,
        port: LocalPortId,
    ) -> Result<type_c_interface::control::svid::DiscoveredSvids, PdError> {
        self.fn_calls
            .push_back(ControllerFnCall::Pd(FnCall::GetDiscoveredSvids(port)));
        self.next_result_get_discovered_svids
            .pop_front()
            .expect("next_result_get_discovered_svids not set")
    }

    async fn get_discover_identity_sop_response(
        &mut self,
        port: LocalPortId,
    ) -> Result<embedded_usb_pd::vdm::structured::command::discover_identity::sop::ResponseVdos, PdError> {
        self.fn_calls
            .push_back(ControllerFnCall::Pd(FnCall::GetDiscoverIdentitySopResponse(port)));
        self.next_result_get_discover_identity_sop_response
            .pop_front()
            .expect("next_result_get_discover_identity_sop_response not set")
    }

    async fn get_discover_identity_sop_prime_response(
        &mut self,
        port: LocalPortId,
    ) -> Result<embedded_usb_pd::vdm::structured::command::discover_identity::sop_prime::ResponseVdos, PdError> {
        self.fn_calls
            .push_back(ControllerFnCall::Pd(FnCall::GetDiscoverIdentitySopPrimeResponse(port)));
        self.next_result_get_discover_identity_sop_prime_response
            .pop_front()
            .expect("next_result_get_discover_identity_sop_prime_response not set")
    }
}
