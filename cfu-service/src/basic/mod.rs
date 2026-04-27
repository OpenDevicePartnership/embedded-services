//! Basic CFU implementation over the basic [`FwUpdate`] trait.
use crate::{
    basic::{
        config::Updater as Config,
        event_receiver::Event,
        state::{FwUpdateState, SharedState},
    },
    component::{InternalResponseData, RequestData},
    customization::Customization,
};
use embedded_cfu_protocol::protocol_definitions::*;
use embedded_services::{debug, error, sync::Lockable};
use fw_update_interface::basic::FwUpdate;

pub mod config;
pub mod event_receiver;
pub mod state;

#[cfg(test)]
mod test;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Output {
    CfuResponse(InternalResponseData),
    CfuRecovery,
}

/// Basic CFU handler that bridges CFU protocol commands with the [`FwUpdate`] trait.
///
/// This struct is generic over a firmware offer validator and processes CFU commands
/// by delegating firmware operations to any [`FwUpdate`] implementor passed to each method.
pub struct Updater<'a, Device: Lockable<Inner: FwUpdate>, Shared: Lockable<Inner = SharedState>, Cust: Customization> {
    device: &'a Device,
    component_id: ComponentId,
    customization: Cust,
    shared_state: &'a Shared,
    config: Config,
}

impl<'a, Device: Lockable<Inner: FwUpdate>, Shared: Lockable<Inner = SharedState>, Cust: Customization>
    Updater<'a, Device, Shared, Cust>
{
    /// Create a new CfuBasic instance
    pub fn new(
        device: &'a Device,
        shared_state: &'a Shared,
        config: Config,
        component_id: ComponentId,
        customization: Cust,
    ) -> Self {
        Self {
            device,
            shared_state,
            component_id,
            customization,
            config,
        }
    }

    /// Create a response with an invalid firmware version
    fn create_invalid_fw_version_response(&self) -> InternalResponseData {
        let dev_inf = FwVerComponentInfo::new(FwVersion::new(0xffffffff), self.component_id);
        let comp_info: [FwVerComponentInfo; MAX_CMPT_COUNT] = [dev_inf; MAX_CMPT_COUNT];
        InternalResponseData::FwVersionResponse(GetFwVersionResponse {
            header: GetFwVersionResponseHeader::new(1, GetFwVerRespHeaderByte3::NoSpecialFlags),
            component_info: comp_info,
        })
    }

    /// Create an offer rejection response
    fn create_offer_rejection() -> InternalResponseData {
        InternalResponseData::OfferResponse(FwUpdateOfferResponse::new_with_failure(
            HostToken::Driver,
            OfferRejectReason::InvalidComponent,
            OfferStatus::Reject,
        ))
    }

    /// Returns a copy of the current update state
    pub async fn update_state(&self) -> FwUpdateState {
        self.shared_state.lock().await.fw_update_state
    }

    /// Gives immutable access to the customization object
    pub fn customization(&self) -> &Cust {
        &self.customization
    }

    /// Gives mutable access to the customization object
    pub fn customization_mut(&mut self) -> &mut Cust {
        &mut self.customization
    }

    /// Process a CFU event
    pub async fn process_event(&mut self, event: Event) -> Output {
        match event {
            Event::Request(request) => {
                let response = self.process_cfu_command(&request).await;
                Output::CfuResponse(response)
            }
            Event::RecoveryTick => {
                // FW Update recovery tick, process recovery attempts
                self.process_recovery_tick().await;
                Output::CfuRecovery
            }
        }
    }

    /// Process a GetFwVersion command
    pub async fn process_get_fw_version(&mut self) -> InternalResponseData {
        let result = self.device.lock().await.get_active_fw_version().await;
        let version = match result {
            Ok(v) => v,
            Err(e) => {
                error!("Failed to get active firmware version: {:?}", e);
                return self.create_invalid_fw_version_response();
            }
        };

        let dev_inf = FwVerComponentInfo::new(FwVersion::new(version), self.component_id);
        let comp_info: [FwVerComponentInfo; MAX_CMPT_COUNT] = [dev_inf; MAX_CMPT_COUNT];
        InternalResponseData::FwVersionResponse(GetFwVersionResponse {
            header: GetFwVersionResponseHeader::new(1, GetFwVerRespHeaderByte3::NoSpecialFlags),
            component_info: comp_info,
        })
    }

    /// Process a GiveOffer command
    pub async fn process_give_offer(&mut self, offer: &FwUpdateOffer) -> InternalResponseData {
        if offer.component_info.component_id != self.component_id {
            return Self::create_offer_rejection();
        }

        let result = self.device.lock().await.get_active_fw_version().await;
        let version = match result {
            Ok(v) => v,
            Err(e) => {
                error!("Failed to get active firmware version: {:?}", e);
                return Self::create_offer_rejection();
            }
        };

        InternalResponseData::OfferResponse(self.customization.validate(FwVersion::new(version), offer))
    }

    /// Process an AbortUpdate command
    pub async fn process_abort_update(&mut self) -> InternalResponseData {
        let result = self.device.lock().await.abort_fw_update().await;
        match result {
            Ok(_) => {
                debug!("FW update aborted successfully");
                self.shared_state.lock().await.enter_idle();
            }
            Err(e) => {
                error!("Failed to abort FW update: {:?}", e);
                self.shared_state.lock().await.enter_recovery();
            }
        }

        InternalResponseData::ComponentPrepared
    }

    /// Process a GiveContent command
    pub async fn process_give_content(&mut self, content: &FwUpdateContentCommand) -> InternalResponseData {
        let data = if let Some(data) = content.data.get(0..content.header.data_length as usize) {
            data
        } else {
            return InternalResponseData::ContentResponse(FwUpdateContentResponse::new(
                content.header.sequence_num,
                CfuUpdateContentResponseStatus::ErrorPrepare,
            ));
        };

        debug!("Got content {:#?}", content);
        if content.header.flags & FW_UPDATE_FLAG_FIRST_BLOCK != 0 {
            debug!("Got first block");

            let result = self.device.lock().await.start_fw_update().await;
            match result {
                Ok(_) => {
                    debug!("FW update started successfully");
                }
                Err(e) => {
                    error!("Failed to start FW update: {:?}", e);
                    self.shared_state.lock().await.enter_recovery();
                    return InternalResponseData::ContentResponse(FwUpdateContentResponse::new(
                        content.header.sequence_num,
                        CfuUpdateContentResponseStatus::ErrorPrepare,
                    ));
                }
            }

            self.shared_state
                .lock()
                .await
                .enter_in_progress(self.config.recovery.tick_interval);
        }

        let result = self
            .device
            .lock()
            .await
            .write_fw_contents(content.header.firmware_address as usize, data)
            .await;
        match result {
            Ok(_) => {
                debug!("Block written successfully");
            }
            Err(e) => {
                error!("Failed to write block: {:?}", e);
                return InternalResponseData::ContentResponse(FwUpdateContentResponse::new(
                    content.header.sequence_num,
                    CfuUpdateContentResponseStatus::ErrorWrite,
                ));
            }
        }

        if content.header.flags & FW_UPDATE_FLAG_LAST_BLOCK != 0 {
            let result = self.device.lock().await.finalize_fw_update().await;
            match result {
                Ok(_) => {
                    debug!("FW update finalized successfully");
                    self.shared_state.lock().await.enter_idle();
                }
                Err(e) => {
                    error!("Failed to finalize FW update: {:?}", e);
                    self.shared_state.lock().await.enter_recovery();
                    return InternalResponseData::ContentResponse(FwUpdateContentResponse::new(
                        content.header.sequence_num,
                        CfuUpdateContentResponseStatus::ErrorWrite,
                    ));
                }
            }
        }

        InternalResponseData::ContentResponse(FwUpdateContentResponse::new(
            content.header.sequence_num,
            CfuUpdateContentResponseStatus::Success,
        ))
    }

    /// Process a CFU recovery tick.
    pub async fn process_recovery_tick(&mut self) {
        // Update timed out, attempt to abort
        self.shared_state.lock().await.enter_recovery();
        let result = self.device.lock().await.abort_fw_update().await;
        match result {
            Ok(_) => {
                debug!("FW update aborted successfully");
                self.shared_state.lock().await.enter_idle();
            }
            Err(e) => {
                error!("Failed to abort FW update: {:?}", e);
            }
        }
    }

    /// Process a CFU command, dispatching to the appropriate handler
    pub async fn process_cfu_command(&mut self, command: &RequestData) -> InternalResponseData {
        let fw_update_state = self.shared_state.lock().await.fw_update_state;
        if fw_update_state == FwUpdateState::Recovery {
            debug!("FW update in recovery state, rejecting command");
            return InternalResponseData::ComponentBusy;
        }

        match command {
            RequestData::FwVersionRequest => {
                debug!("Got FwVersionRequest");
                self.process_get_fw_version().await
            }
            RequestData::GiveOffer(offer) => {
                debug!("Got GiveOffer");
                self.process_give_offer(offer).await
            }
            RequestData::GiveContent(content) => {
                debug!("Got GiveContent");
                self.process_give_content(content).await
            }
            RequestData::AbortUpdate => {
                debug!("Got AbortUpdate");
                self.process_abort_update().await
            }
            RequestData::FinalizeUpdate => {
                debug!("Got FinalizeUpdate");
                InternalResponseData::ComponentPrepared
            }
            RequestData::PrepareComponentForUpdate => {
                debug!("Got PrepareComponentForUpdate");
                InternalResponseData::ComponentPrepared
            }
            RequestData::GiveOfferExtended(_) => {
                debug!("Got GiveExtendedOffer, rejecting");
                Self::create_offer_rejection()
            }
            RequestData::GiveOfferInformation(_) => {
                debug!("Got GiveOfferInformation, rejecting");
                Self::create_offer_rejection()
            }
        }
    }
}
