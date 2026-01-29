use super::{ControllerWrapper, FwOfferValidator};
use crate::wrapper::message::OutputDpStatusChanged;
use embassy_sync::blocking_mutex::raw::RawMutex;
use embedded_services::{sync::Lockable, trace, type_c::controller::Controller};
use embedded_usb_pd::{Error, LocalPortId};

impl<'device, M: RawMutex, C: Lockable, V: FwOfferValidator, const POLICY_CHANNEL_SIZE: usize>
    ControllerWrapper<'device, M, C, V, POLICY_CHANNEL_SIZE>
where
    <C as Lockable>::Inner: Controller,
{
    /// Process a DisplayPort status update by retrieving the current DP status from the `controller` for the appropriate `port`.
    pub(super) async fn process_dp_status_update(
        &self,
        controller: &mut C::Inner,
        port: LocalPortId,
    ) -> Result<OutputDpStatusChanged, Error<<C::Inner as Controller>::BusError>> {
        trace!("Processing DP status update event on port {}", port.0);

        let status = controller.get_dp_status(port).await?;
        Ok(OutputDpStatusChanged { port, status })
    }
}
