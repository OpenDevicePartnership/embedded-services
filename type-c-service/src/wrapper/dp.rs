use super::{backing::Backing, ControllerWrapper, FwOfferValidator};
use crate::wrapper::message::OutputDpStatusChanged;
use embedded_services::{trace, type_c::controller::Controller};
use embedded_usb_pd::{Error, LocalPortId};

impl<'a, const N: usize, C: Controller, BACK: Backing<'a>, V: FwOfferValidator> ControllerWrapper<'a, N, C, BACK, V> {
    /// Process a DisplayPort status update by retrieving the current DP status from the `controller` for the appropriate `port`.
    pub(super) async fn process_dp_status_update(
        &self,
        controller: &mut C,
        port: LocalPortId,
    ) -> Result<OutputDpStatusChanged, Error<<C as Controller>::BusError>> {
        trace!("Processing DP status update event on port {}", port.0);

        let status = controller.get_dp_status(port).await?;
        Ok(OutputDpStatusChanged { port, status })
    }
}
