//! Device state machine actions
use embedded_cfu_protocol::protocol_definitions::{
    CfuProtocolError, FwUpdateContentCommand, FwUpdateOfferCommand, FwUpdateOfferResponse,
};

use super::*;
use crate::cfu::component::{self, CfuDevice, InternalResponseData, InternalState};
use crate::cfu::{self, CfuError};
use crate::info;

/// Device state machine control
pub struct Device<'a, S: Kind> {
    device: &'a CfuDevice,
    _state: core::marker::PhantomData<S>,
}

/// Enum to contain any state
pub enum AnyState<'a> {
    /// Idle
    Idle(Device<'a, Idle>),
    /// Ready
    Ready(Device<'a, Ready>),
    /// Busy
    Busy(Device<'a, Busy>),
    /// Finalizing an update
    FinalizingUpdate(Device<'a, FinalizingUpdate>),
}

impl AnyState<'_> {
    /// Return the kind of the contained state
    pub fn kind(&self) -> ComponentState {
        match self {
            AnyState::Busy(_) => ComponentState::Busy,
            AnyState::Idle(_) => ComponentState::Idle,
            AnyState::FinalizingUpdate(_) => ComponentState::FinalizingUpdate,
            AnyState::Ready(_) => ComponentState::Ready,
        }
    }
}

impl<'a, S: Kind> Device<'a, S> {
    /// Create a new state machine
    pub(crate) fn new(device: &'a CfuDevice) -> Self {
        Self {
            device,
            _state: core::marker::PhantomData,
        }
    }
    /// Something went wrong (bad image, bad signature, etc) during an update and device needs to restore idle state
    pub async fn bail(&self) -> Result<Device<'a, Idle>, CfuError> {
        info!("Component {} needs to stop updating", self.device.component_id());
        self.device.set_state(InternalState::new(ComponentState::Idle)).await;
        Ok(Device::new(self.device))
    }
}

impl<'a> Device<'a, FinalizingUpdate> {
    /// Done finalizing, report the component has finished and is idle once more
    pub async fn finish_component_update(self) -> Result<Device<'a, Idle>, CfuError> {
        info!("Received attach from device {}", self.device.component_id());
        self.device.set_state(InternalState::new(ComponentState::Idle)).await;
        self.device
            .send_response(cfu::component::InternalResponseData::ComponentPrepared)
            .await;
        Ok(Device::new(self.device))
    }
}

impl<'a> Device<'a, Busy> {
    /// Done finalizing, report the component has finished and is idle once more
    pub async fn finalize_update(self) -> Result<Device<'a, FinalizingUpdate>, CfuError> {
        info!(
            "Update complete, running finalize logic on component {}",
            self.device.component_id()
        );
        self.device
            .set_state(InternalState::new(ComponentState::FinalizingUpdate))
            .await;
        Ok(Device::new(self.device))
    }

    /// Continue processing content chunks for update
    pub async fn receive_next_content_chunk(
        self,
        chunk: FwUpdateContentCommand<&'static [u8]>,
    ) -> Result<(), CfuError> {
        info!("r");
        let resp = self
            .device
            .execute_device_request(component::RequestData::GiveContent(chunk))
            .await
            .map_err(CfuError::ProtocolError)?;
        self.device.send_response(resp).await;
        Err(CfuError::BadImage)
    }
}

impl<'a> Device<'a, Idle> {
    /// Done finalizing, report the component has finished and is idle once more
    pub async fn prepare_component(self) -> Result<Device<'a, Ready>, CfuError> {
        info!("Received offer for component {}", self.device.component_id());
        self.device.set_state(InternalState::new(ComponentState::Busy)).await;
        match self
            .device
            .execute_device_request(component::RequestData::PrepareComponentForUpdate)
            .await
            .map_err(CfuError::ProtocolError)?
        {
            InternalResponseData::PrimaryNeedsSubcomponentsPrepared(ids) => {
                if self.device.state().await.waiting_on_subs {
                    // This must be second time receiving the prepare component request for this component
                    // After first request, all subcomponents should have been prepped, component is now ready to receive offers
                    self.device.set_state(InternalState::new(ComponentState::Ready)).await;
                    self.device.send_response(InternalResponseData::ComponentPrepared).await;
                } else {
                    self.device
                        .set_state(InternalState::new_with_subcomponent_info(
                            self.device.state().await.state,
                            true,
                        ))
                        .await;
                    self.device
                        .send_response(InternalResponseData::PrimaryNeedsSubcomponentsPrepared(ids))
                        .await;
                }
            }
            InternalResponseData::ComponentPrepared => {}
            _ => return Err(CfuError::ProtocolError(CfuProtocolError::BadResponse)),
        }
        Ok(Device::new(self.device))
    }
}

impl<'a> Device<'a, Ready> {
    /// Component device needs to accept offer and transition to Busy state to receive content
    pub async fn accept_offer(self) -> Device<'a, Busy> {
        info!("Accepting offer for component {}", self.device.component_id());
        self.device.set_state(InternalState::new(ComponentState::Busy)).await;
        Device::new(self.device)
    }
    /// Component device needs to reject offer and stay ready for another offer
    pub async fn reject_offer(&self) {
        info!("Rejecting offer for component {}", self.device.component_id());
    }
    /// Component device must validate the offer and see if it's applicable
    pub async fn evaluate_offer(self, offer: FwUpdateOfferCommand) -> Result<FwUpdateOfferResponse, CfuError> {
        info!("Evaluating offer for component {}", self.device.component_id());
        match self
            .device
            .execute_device_request(component::RequestData::GiveOffer(offer))
            .await
            .map_err(CfuError::ProtocolError)?
        {
            InternalResponseData::ComponentBusy => Err(CfuError::ComponentBusy),
            InternalResponseData::OfferResponse(r) => Ok(r),

            _ => Err(CfuError::ProtocolError(CfuProtocolError::BadResponse)),
        }
    }
}
