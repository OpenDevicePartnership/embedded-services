use super::{ControllerWrapper, FwOfferValidator};
use crate::wrapper::message::OutputDpStatusChanged;
use embassy_sync::blocking_mutex::raw::RawMutex;
use embedded_services::{trace, type_c::controller::Controller};
use embedded_usb_pd::{Error, LocalPortId};

impl<'a, M: RawMutex, C: Controller, V: FwOfferValidator> ControllerWrapper<'a, M, C, V> {
    /// Process a DisplayPort status update by retrieving the current DP status from the `controller` for the appropriate `port`.
    pub(super) async fn process_dp_status_update(
        &self,
        controller: &mut C,
        port: LocalPortId,
    ) -> Result<OutputDpStatusChanged, Error<<C as Controller>::BusError>> {
        trace!("Processing DP status update event on {:?}", port);

        let status = controller.get_dp_status(port).await?;
        Ok(OutputDpStatusChanged { port, status })
    }
}
