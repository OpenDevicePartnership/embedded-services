use super::{ControllerWrapper, FwOfferValidator};
use crate::type_c::controller::Controller;
use crate::wrapper::message::OutputDpStatusChanged;
use embassy_sync::blocking_mutex::raw::RawMutex;
use embedded_services::{event, sync::Lockable, trace};
use embedded_usb_pd::{Error, LocalPortId};

impl<
    'device,
    M: RawMutex,
    D: Lockable,
    S: event::Sender<power_policy_service::psu::event::EventData>,
    V: FwOfferValidator,
> ControllerWrapper<'device, M, D, S, V>
where
    D::Inner: Controller,
{
    /// Process a DisplayPort status update by retrieving the current DP status from the `controller` for the appropriate `port`.
    pub(super) async fn process_dp_status_update(
        &self,
        controller: &mut D::Inner,
        port: LocalPortId,
    ) -> Result<OutputDpStatusChanged, Error<<D::Inner as Controller>::BusError>> {
        trace!("Processing DP status update event on port {}", port.0);

        let status = controller.get_dp_status(port).await?;
        Ok(OutputDpStatusChanged { port, status })
    }
}
