//! VDM (Vendor Defined Messages) related functionality.

use embedded_services::type_c::controller::{AttnVdm, OtherVdm};
use embedded_usb_pd::{GlobalPortId, PdError};

use super::Service;

impl Service<'_> {
    /// Get the other vdm for the given port
    pub async fn get_other_vdm(&self, port_id: GlobalPortId) -> Result<OtherVdm, PdError> {
        self.context.get_other_vdm(port_id).await
    }

    /// Get the attention vdm for the given port
    pub async fn get_attn_vdm(&self, port_id: GlobalPortId) -> Result<AttnVdm, PdError> {
        self.context.get_attn_vdm(port_id).await
    }
}
