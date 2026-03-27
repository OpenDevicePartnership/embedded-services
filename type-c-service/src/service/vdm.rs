//! VDM (Vendor Defined Messages) related functionality.

use embedded_services::event::Receiver;
use embedded_usb_pd::{GlobalPortId, PdError};
use power_policy_interface::service::event::EventData as PowerPolicyEventData;
use type_c_interface::port::{AttnVdm, OtherVdm};

use super::Service;

impl<'a, PowerReceiver: Receiver<PowerPolicyEventData>> Service<'a, PowerReceiver> {
    /// Get the other vdm for the given port
    pub async fn get_other_vdm(&self, port_id: GlobalPortId) -> Result<OtherVdm, PdError> {
        self.context.get_other_vdm(port_id).await
    }

    /// Get the attention vdm for the given port
    pub async fn get_attn_vdm(&self, port_id: GlobalPortId) -> Result<AttnVdm, PdError> {
        self.context.get_attn_vdm(port_id).await
    }
}
