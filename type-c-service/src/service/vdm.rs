//! VDM (Vendor Defined Messages) related functionality.

use type_c_interface::port::{AttnVdm, OtherVdm};
use embedded_services::sync::Lockable;
use embedded_usb_pd::{GlobalPortId, PdError};
use power_policy_interface::psu;

use super::Service;

impl<PSU: Lockable> Service<'_, PSU>
where
    PSU::Inner: psu::Psu,
{
    /// Get the other vdm for the given port
    pub async fn get_other_vdm(&self, port_id: GlobalPortId) -> Result<OtherVdm, PdError> {
        self.context.get_other_vdm(port_id).await
    }

    /// Get the attention vdm for the given port
    pub async fn get_attn_vdm(&self, port_id: GlobalPortId) -> Result<AttnVdm, PdError> {
        self.context.get_attn_vdm(port_id).await
    }
}
