//! CFU message bridge
//! TODO: remove this once we have a more generic FW update implementation
use embassy_futures::select::{Either, select};
use embedded_cfu_protocol::protocol_definitions::*;
use embedded_services::cfu::component::{InternalResponseData, RequestData};
use embedded_services::power;
use embedded_services::type_c::controller::Controller;
use embedded_services::{debug, error};

use super::message::EventCfu;
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

impl<'a, M: RawMutex, C: Controller, V: FwOfferValidator> ControllerWrapper<'a, M, C, V> {
    /// Create a new invalid FW version response
    fn create_invalid_fw_version_response(&self) -> InternalResponseData {
        let dev_inf = FwVerComponentInfo::new(FwVersion::new(0xffffffff), self.registration.cfu_device.component_id());
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

        let dev_inf = FwVerComponentInfo::new(FwVersion::new(version), self.registration.cfu_device.component_id());
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
        if offer.component_info.component_id != self.registration.cfu_device.component_id() {
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

    async fn process_abort_update(&self, controller: &mut C, state: &mut dyn DynPortState<'_>) -> InternalResponseData {
        // abort the update process
        match controller.abort_fw_update().await {
            Ok(_) => {
                debug!("FW update aborted successfully");
                state.controller_state_mut().fw_update_state = FwUpdateState::Idle;
            }
            Err(Error::Pd(e)) => {
                error!("Failed to abort FW update: {:?}", e);
                state.controller_state_mut().fw_update_state = FwUpdateState::Recovery;
            }
            Err(Error::Bus(_)) => {
                error!("Failed to abort FW update, bus error");
                state.controller_state_mut().fw_update_state = FwUpdateState::Recovery;
            }
        }

        InternalResponseData::ComponentPrepared
    }

    /// Process a GiveContent command
    async fn process_give_content(
        &self,
        controller: &mut C,
        state: &mut dyn DynPortState<'_>,
        content: &FwUpdateContentCommand,
    ) -> InternalResponseData {
        let data = &content.data[0..content.header.data_length as usize];
        debug!("Got content {:#?}", content);
        if content.header.flags & FW_UPDATE_FLAG_FIRST_BLOCK != 0 {
            debug!("Got first block");

            // Detach from the power policy so it doesn't attempt to do anything while we are updating
            let controller_id = self.registration.pd_controller.id();
            let mut detached_all = true;
            for power in self.registration.power_devices {
                info!("{}: checking power device", controller_id);
                if power.state().await != power::policy::device::State::Detached {
                    info!("{}: Detaching power device", controller_id);
                    if let Err(e) = power.detach().await {
                        error!("{}: Failed to detach power device: {:?}", controller_id, e);

                        // Sync to bring the controller to a known state with all services
                        match self.sync_state_internal(controller, state).await {
                            Ok(_) => debug!(
                                "{}: Synced state after detaching power device",
                                controller_id
                            ),
                            Err(Error::Pd(e)) => error!(
                                "{}: Failed to sync state after detaching power device: {:?}",
                                controller_id, e
                            ),
                            Err(Error::Bus(_)) => error!(
                                "{}: Failed to sync state after detaching power device, bus error",
                                controller_id
                            ),
                        }

                        detached_all = false;
                        break;
                    }
                }
            }

            if !detached_all {
                error!(
                    "{}: Failed to detach all power devices, rejecting offer",
                    controller_id
                );
                return InternalResponseData::ContentResponse(FwUpdateContentResponse::new(
                    content.header.sequence_num,
                    CfuUpdateContentResponseStatus::ErrorPrepare,
                ));
            }

            // Need to start the update
            self.fw_update_ticker.lock().await.reset();
            match controller.start_fw_update().await {
                Ok(_) => {
                    debug!("FW update started successfully");
                }
                Err(Error::Pd(e)) => {
                    error!("Failed to start FW update: {:?}", e);
                    state.controller_state_mut().fw_update_state = FwUpdateState::Recovery;
                    return InternalResponseData::ContentResponse(FwUpdateContentResponse::new(
                        content.header.sequence_num,
                        CfuUpdateContentResponseStatus::ErrorPrepare,
                    ));
                }
                Err(Error::Bus(_)) => {
                    error!("Failed to start FW update, bus error");
                    state.controller_state_mut().fw_update_state = FwUpdateState::Recovery;
                    return InternalResponseData::ContentResponse(FwUpdateContentResponse::new(
                        content.header.sequence_num,
                        CfuUpdateContentResponseStatus::ErrorPrepare,
                    ));
                }
            }

            state.controller_state_mut().fw_update_state = FwUpdateState::InProgress(0);
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
                    state.controller_state_mut().fw_update_state = FwUpdateState::Idle;
                }
                Err(Error::Pd(e)) => {
                    error!("Failed to finalize FW update: {:?}", e);
                    state.controller_state_mut().fw_update_state = FwUpdateState::Recovery;
                    return Self::create_offer_rejection();
                }
                Err(Error::Bus(_)) => {
                    error!("Failed to finalize FW update, bus error");
                    state.controller_state_mut().fw_update_state = FwUpdateState::Recovery;
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
    pub async fn process_cfu_tick(&self, controller: &mut C, state: &mut dyn DynPortState<'_>) {
        match state.controller_state_mut().fw_update_state {
            FwUpdateState::Idle => {
                // No FW update in progress, nothing to do
                return;
            }
            FwUpdateState::InProgress(ticks) => {
                if ticks + 1 < DEFAULT_FW_UPDATE_TIMEOUT_TICKS {
                    trace!("CFU tick: {}", ticks);
                    state.controller_state_mut().fw_update_state = FwUpdateState::InProgress(ticks + 1);
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
        state.controller_state_mut().fw_update_state = FwUpdateState::Recovery;
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

        state.controller_state_mut().fw_update_state = FwUpdateState::Idle;
    }

    /// Process a CFU command
    pub async fn process_cfu_command(
        &self,
        controller: &mut C,
        state: &mut dyn DynPortState<'_>,
        command: &RequestData,
    ) -> InternalResponseData {
        if state.controller_state().fw_update_state == FwUpdateState::Recovery {
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
            RequestData::AbortUpdate => {
                debug!("Got AbortUpdate");
                self.process_abort_update(controller, state).await
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

    /// Sends a CFU response to the command
    pub async fn send_cfu_response(&self, response: InternalResponseData) {
        self.registration.cfu_device.send_response(response).await;
    }

    /// Wait for a CFU command
    ///
    /// Returns None if the FW update ticker has ticked
    /// DROP SAFETY: No state that needs to be restored
    pub async fn wait_cfu_command(&self) -> EventCfu {
        // Only lock long enough to grab our state
        let fw_update_state = self.state.lock().await.controller_state().fw_update_state;
        match fw_update_state {
            FwUpdateState::Idle => {
                // No FW update in progress, just wait for a command
                EventCfu::Request(self.registration.cfu_device.wait_request().await)
            }
            FwUpdateState::InProgress(_) => {
                match select(
                    self.registration.cfu_device.wait_request(),
                    self.fw_update_ticker.lock().await.next(),
                )
                .await
                {
                    Either::First(command) => EventCfu::Request(command),
                    Either::Second(_) => {
                        debug!("FW update ticker ticked");
                        EventCfu::RecoveryTick
                    }
                }
            }
            FwUpdateState::Recovery => {
                // Recovery state, wait for the next attempt to recover the device
                self.fw_update_ticker.lock().await.next().await;
                debug!("FW update ticker ticked");
                EventCfu::RecoveryTick
            }
        }
    }
}
