//! Mock implementation of [`type_c_interface::controller::max_sink_voltage::MaxSinkVoltage`]

use embedded_usb_pd::{LocalPortId, PdError};
use type_c_interface::controller::max_sink_voltage::MaxSinkVoltage;

use super::FnCall as ControllerFnCall;
use super::Mock;

/// Contains a [`MaxSinkVoltage`] function call and its arguments
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FnCall {
    SetMaxSinkVoltage(LocalPortId, Option<u16>),
}

impl MaxSinkVoltage for Mock {
    async fn set_max_sink_voltage(&mut self, port: LocalPortId, voltage_mv: Option<u16>) -> Result<(), PdError> {
        self.fn_calls
            .push_back(ControllerFnCall::MaxSinkVoltage(FnCall::SetMaxSinkVoltage(
                port, voltage_mv,
            )));
        self.next_result_set_max_sink_voltage
            .pop_front()
            .expect("next_result_set_max_sink_voltage not set")
    }
}
