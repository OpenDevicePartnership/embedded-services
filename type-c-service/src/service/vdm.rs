//! VDM (Vendor Defined Messages) related functionality.

use crate::type_c::controller::{AttnVdm, OtherVdm};
use embedded_services::{intrusive_list, sync::Lockable};
use embedded_usb_pd::{GlobalPortId, PdError};
use power_policy_service::psu;

use super::Service;

impl<PSU: Lockable> Service<'_, PSU>
where
    PSU::Inner: psu::Psu,
{
    /// Get the other vdm for the given port
    pub async fn get_other_vdm(
        &self,
        controllers: &intrusive_list::IntrusiveList,
        port_id: GlobalPortId,
    ) -> Result<OtherVdm, PdError> {
        self.context.get_other_vdm(controllers, port_id).await
    }

    /// Get the attention vdm for the given port
    pub async fn get_attn_vdm(
        &self,
        controllers: &intrusive_list::IntrusiveList,
        port_id: GlobalPortId,
    ) -> Result<AttnVdm, PdError> {
        self.context.get_attn_vdm(controllers, port_id).await
    }
}
