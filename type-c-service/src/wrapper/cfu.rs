//! CFU message bridge
//! TODO: remove this once we have a more generic FW update implementation
use embassy_futures::select::{select, Either};
use embedded_cfu_protocol::protocol_definitions::*;
use embedded_services::cfu::component::{InternalResponseData, RequestData};
use embedded_services::type_c::controller::Controller;
use embedded_services::{debug, error};

use super::*;

/// Current state of the firmware update process
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FwUpdateState {
    /// None in progress
    Idle,
    /// Firmware update in progress
    /// Contains number of ticks [`super::DEFAULT_FW_UPDATE_TICK_INTERVAL_MS`] that have passed
    InProgress(u8),
    /// Firmware update has failed and the device is in an unknown state
    Recovery,
}

impl FwUpdateState {
    /// Check if the firmware update is in progress
    pub fn in_progress(&self) -> bool {
        matches!(self, FwUpdateState::InProgress(_) | FwUpdateState::Recovery)
    }
}

impl<const N: usize, C: Controller, V: FwOfferValidator> ControllerWrapper<'_, N, C, V> {
    /// Create a new invalid FW version response
    fn create_invalid_fw_version_response(&self) -> InternalResponseData {
        let dev_inf = FwVerComponentInfo::new(FwVersion::new(0xffffffff), self.cfu_device.component_id());
        let comp_info: [FwVerComponentInfo; MAX_CMPT_COUNT] = [dev_inf; MAX_CMPT_COUNT];
        InternalResponseData::FwVersionResponse(GetFwVersionResponse {
            header: GetFwVersionResponseHeader::new(1, GetFwVerRespHeaderByte3::NoSpecialFlags),
            component_info: comp_info,
        })
    }

    /// Process a GetFwVersion command
    async fn process_get_fw_version(&self, target: &mut C) -> InternalResponseData {
        let version = match target.get_active_fw_version().await {
            Ok(v) => v,
            Err(Error::Pd(e)) => {
                error!("Failed to get active firmware version: {:?}", e);
                return self.create_invalid_fw_version_response();
            }
            Err(Error::Bus(_)) => {
                error!("Failed to get active firmware version, bus error");
                return self.create_invalid_fw_version_response();
            }
        };

        let dev_inf = FwVerComponentInfo::new(FwVersion::new(version), self.cfu_device.component_id());
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

    /// Process a GiveOffer command
    async fn process_give_offer(&self, target: &mut C, offer: &FwUpdateOffer) -> InternalResponseData {
        if offer.component_info.component_id != self.cfu_device.component_id() {
            return Self::create_offer_rejection();
        }

        let version = match target.get_active_fw_version().await {
            Ok(v) => v,
            Err(Error::Pd(e)) => {
                error!("Failed to get active firmware version: {:?}", e);
                return Self::create_offer_rejection();
            }
            Err(Error::Bus(_)) => {
                error!("Failed to get active firmware version, bus error");
                return Self::create_offer_rejection();
            }
        };

        InternalResponseData::OfferResponse(self.fw_version_validator.validate(FwVersion::new(version), offer))
    }

    /// Process a GiveContent command
    async fn process_give_content(
        &self,
        controller: &mut C,
        state: &mut InternalState,
        content: &FwUpdateContentCommand,
    ) -> InternalResponseData {
        let data = &content.data[0..content.header.data_length as usize];
        debug!("Got content {:#?}", content);
        if content.header.flags & FW_UPDATE_FLAG_FIRST_BLOCK != 0 {
            debug!("Got first block");

            // Need to start the update
            state.fw_update_ticker.reset();
            match controller.start_fw_update().await {
                Ok(_) => {
                    debug!("FW update started successfully");
                }
                Err(Error::Pd(e)) => {
                    error!("Failed to start FW update: {:?}", e);
                    state.fw_update_state = FwUpdateState::Recovery;
                    return InternalResponseData::ContentResponse(FwUpdateContentResponse::new(
                        content.header.sequence_num,
                        CfuUpdateContentResponseStatus::ErrorPrepare,
                    ));
                }
                Err(Error::Bus(_)) => {
                    error!("Failed to start FW update, bus error");
                    state.fw_update_state = FwUpdateState::Recovery;
                    return InternalResponseData::ContentResponse(FwUpdateContentResponse::new(
                        content.header.sequence_num,
                        CfuUpdateContentResponseStatus::ErrorPrepare,
                    ));
                }
            }

            state.fw_update_state = FwUpdateState::InProgress(0);
        }

        match controller
            .write_fw_contents(content.header.firmware_address as usize, data)
            .await
        {
            Ok(_) => {
                debug!("Block written successfully");
            }
            Err(Error::Pd(e)) => {
                error!("Failed to write block: {:?}", e);
                return InternalResponseData::ContentResponse(FwUpdateContentResponse::new(
                    content.header.sequence_num,
                    CfuUpdateContentResponseStatus::ErrorWrite,
                ));
            }
            Err(Error::Bus(_)) => {
                error!("Failed to write block, bus error");
                return InternalResponseData::ContentResponse(FwUpdateContentResponse::new(
                    content.header.sequence_num,
                    CfuUpdateContentResponseStatus::ErrorWrite,
                ));
            }
        }

        if content.header.flags & FW_UPDATE_FLAG_LAST_BLOCK != 0 {
            match controller.finalize_fw_update().await {
                Ok(_) => {
                    debug!("FW update finalized successfully");
                    state.fw_update_state = FwUpdateState::Idle;
                }
                Err(Error::Pd(e)) => {
                    error!("Failed to finalize FW update: {:?}", e);
                    state.fw_update_state = FwUpdateState::Recovery;
                    return Self::create_offer_rejection();
                }
                Err(Error::Bus(_)) => {
                    error!("Failed to finalize FW update, bus error");
                    state.fw_update_state = FwUpdateState::Recovery;
                    return Self::create_offer_rejection();
                }
            }
        }

        InternalResponseData::ContentResponse(FwUpdateContentResponse::new(
            content.header.sequence_num,
            CfuUpdateContentResponseStatus::Success,
        ))
    }

    /// Process a CFU tick
    pub async fn process_cfu_tick(&self, controller: &mut C, state: &mut InternalState) {
        match state.fw_update_state {
            FwUpdateState::Idle => {
                // No FW update in progress, nothing to do
                return;
            }
            FwUpdateState::InProgress(ticks) => {
                if ticks + 1 < DEFAULT_FW_UPDATE_TIMEOUT_TICKS {
                    trace!("CFU tick: {}", ticks);
                    state.fw_update_state = FwUpdateState::InProgress(ticks + 1);
                    return;
                } else {
                    error!("FW update timed out after {} ticks", ticks);
                }
            }
            FwUpdateState::Recovery => {
                // Continue recovery process
            }
        };

        // Update timed out, attempt to exit the FW update
        state.fw_update_state = FwUpdateState::Recovery;
        match controller.abort_fw_update().await {
            Ok(_) => {
                debug!("FW update aborted successfully");
            }
            Err(Error::Pd(e)) => {
                error!("Failed to abort FW update: {:?}", e);
                return;
            }
            Err(Error::Bus(_)) => {
                error!("Failed to abort FW update, bus error");
                return;
            }
        }

        state.fw_update_state = FwUpdateState::Idle;
    }

    /// Process a CFU command
    pub async fn process_cfu_command(
        &self,
        controller: &mut C,
        state: &mut InternalState,
        command: &RequestData,
    ) -> InternalResponseData {
        if state.fw_update_state == FwUpdateState::Recovery {
            debug!("FW update in recovery state, rejecting command");
            return InternalResponseData::ComponentBusy;
        }

        match command {
            RequestData::FwVersionRequest => {
                debug!("Got FwVersionRequest");
                self.process_get_fw_version(controller).await
            }
            RequestData::GiveOffer(offer) => {
                debug!("Got GiveOffer");
                self.process_give_offer(controller, offer).await
            }
            RequestData::GiveContent(content) => {
                debug!("Got GiveContent");
                self.process_give_content(controller, state, content).await
            }
            RequestData::FinalizeUpdate => {
                debug!("Got FinalizeUpdate");
                InternalResponseData::ComponentPrepared
            }
            RequestData::PrepareComponentForUpdate => {
                debug!("Got PrepareComponentForUpdate");
                InternalResponseData::ComponentPrepared
            }
        }
    }

    /// Sends a CFU response to the command
    pub async fn send_cfu_response(&self, response: InternalResponseData) {
        self.cfu_device.send_response(response).await;
    }

    /// Wait for a CFU command
    ///
    /// Returns None if the FW update ticker has ticked
    pub async fn wait_cfu_command(&self, state: &mut InternalState) -> Option<RequestData> {
        match state.fw_update_state {
            FwUpdateState::Idle => {
                // No FW update in progress, just wait for a command
                Some(self.cfu_device.wait_request().await)
            }
            FwUpdateState::InProgress(_) => {
                match select(self.cfu_device.wait_request(), state.fw_update_ticker.next()).await {
                    Either::First(command) => Some(command),
                    Either::Second(_) => {
                        debug!("FW update ticker ticked");
                        None
                    }
                }
            }
            FwUpdateState::Recovery => {
                // Recovery state, wait for the next attempt to recover the device
                state.fw_update_ticker.next().await;
                debug!("FW update ticker ticked");
                None
            }
        }
    }
}
