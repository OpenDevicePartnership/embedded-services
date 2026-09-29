use core::num::NonZeroU8;

use embedded_usb_pd::PdError;

use crate::port::pd::Pd;

pub trait ElectricalDisconnect: Pd {
    /// Execute an electrical disconnect on this port, if supported by the controller.
    ///
    /// If `reconnect_time_s` is provided, the controller should automatically reconnect the port after the specified time
    /// has elapsed. If `reconnect_time_s` is [`None`], the port should remain disconnected until manually reconnected.
    fn execute_electrical_disconnect(
        &mut self,
        reconnect_time_s: Option<NonZeroU8>,
    ) -> impl Future<Output = Result<(), PdError>>;
}
