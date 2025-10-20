//! Power Delivery (PD) related functionality.

use embedded_services::intrusive_list;
use embedded_usb_pd::{GlobalPortId, PdError, ado::Ado};

use super::Service;

impl Service<'_> {
    /// Get the oldest unhandled PD alert for the given port.
    ///
    /// Returns [`None`] if no alerts are pending.
    pub async fn get_pd_alert(
        &self,
        controllers: &intrusive_list::IntrusiveList,
        port: GlobalPortId,
    ) -> Result<Option<Ado>, PdError> {
        self.context.get_pd_alert(controllers, port).await
    }
}
