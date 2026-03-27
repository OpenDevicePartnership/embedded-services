//! Power Delivery (PD) related functionality.

use embedded_services::event::Receiver;
use embedded_usb_pd::{GlobalPortId, PdError, ado::Ado};
use power_policy_interface::service::event::EventData as PowerPolicyEventData;

use super::Service;

impl<'a, PowerReceiver: Receiver<PowerPolicyEventData>> Service<'a, PowerReceiver> {
    /// Get the oldest unhandled PD alert for the given port.
    ///
    /// Returns [`None`] if no alerts are pending.
    pub async fn get_pd_alert(&self, port: GlobalPortId) -> Result<Option<Ado>, PdError> {
        self.context.get_pd_alert(port).await
    }
}
