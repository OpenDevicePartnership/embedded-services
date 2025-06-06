//! Module that can broadcast CFU messages to multiple devices
//! This allows devices to share a single component ID

use core::iter::zip;

use embedded_cfu_protocol::protocol_definitions::*;
use embedded_services::{
    cfu::{
        self,
        component::{CfuDevice, InternalResponseData, RequestData},
    },
    error, intrusive_list, trace,
};

/// Trait containing customization functionality for [`Splitter`]
pub trait Customization {
    /// Decides which firmware version to use based on the provided versions from all devices.
    fn resolve_fw_versions(&self, versions: &[FwVersion]) -> FwVersion;

    /// Decides which offer response to send based on the provided responses from all devices.
    fn resolve_offer_response(&self, offer_responses: &[FwUpdateOfferResponse]) -> FwUpdateOfferResponse;

    /// Decides which content response to send based on the provided responses from all devices.
    fn resolve_content_response(&self, content_responses: &[FwUpdateContentResponse]) -> FwUpdateContentResponse;
}

/// Splitter struct
pub struct Splitter<'a, C: Customization> {
    /// CFU device
    cfu_device: CfuDevice,
    /// Component ID for each individual device
    devices: &'a [ComponentId],
    /// Customization for the Splitter
    customization: C,
}

/// Maximum number of devices supported
pub const MAX_SUPPORTED_DEVICES: usize = 8;

impl<'a, C: Customization> Splitter<'a, C> {
    /// Create a new Splitter
    pub fn new(component_id: ComponentId, devices: &'a [ComponentId], customization: C) -> Self {
        Self {
            cfu_device: CfuDevice::new(component_id),
            devices,
            customization,
        }
    }

    /// Create a new invalid FW version response
    fn create_invalid_fw_version_response(&self) -> InternalResponseData {
        let dev_inf = FwVerComponentInfo::new(FwVersion::new(0xffffffff), self.cfu_device.component_id());
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

    /// Create a content rejection response
    fn create_content_rejection(sequence: u16) -> InternalResponseData {
        InternalResponseData::ContentResponse(FwUpdateContentResponse::new(
            sequence,
            CfuUpdateContentResponseStatus::ErrorInvalid,
        ))
    }

    /// Process a fw version request
    async fn process_get_fw_version(&self) -> InternalResponseData {
        let mut versions = [FwVersion::default(); MAX_SUPPORTED_DEVICES];

        if self.devices.len() > MAX_SUPPORTED_DEVICES {
            error!("More devices than supported");
            return self.create_invalid_fw_version_response();
        }

        for (device_id, version) in zip(self.devices.iter(), versions.iter_mut()) {
            let response = cfu::route_request(*device_id, RequestData::FwVersionRequest).await;
            match response {
                Ok(InternalResponseData::FwVersionResponse(version_info)) => {
                    *version = version_info.component_info[0].fw_version;
                }
                Err(_) | Ok(_) => {
                    error!("Failed to get FW version for device {}", device_id);
                    return self.create_invalid_fw_version_response();
                }
            }
        }

        let overall_version = self.customization.resolve_fw_versions(&versions[..self.devices.len()]);
        let dev_inf = FwVerComponentInfo::new(overall_version, self.cfu_device.component_id());
        let comp_info: [FwVerComponentInfo; MAX_CMPT_COUNT] = [dev_inf; MAX_CMPT_COUNT];
        InternalResponseData::FwVersionResponse(GetFwVersionResponse {
            header: GetFwVersionResponseHeader::new(1, GetFwVerRespHeaderByte3::NoSpecialFlags),
            component_info: comp_info,
        })
    }

    /// Process a give offer request
    async fn process_give_offer(&self, offer: &FwUpdateOffer) -> InternalResponseData {
        let mut offer_responses = [FwUpdateOfferResponse::default(); MAX_SUPPORTED_DEVICES];

        if self.devices.len() > MAX_SUPPORTED_DEVICES {
            error!("More devices than supported");
            return Self::create_offer_rejection();
        }

        for (device_id, offer_response) in zip(self.devices.iter(), offer_responses.iter_mut()) {
            let mut offer = *offer;

            // Override with the correct component ID for the device
            offer.component_info.component_id = *device_id;
            let response = cfu::route_request(*device_id, RequestData::GiveOffer(offer)).await;
            match response {
                Ok(InternalResponseData::OfferResponse(response)) => {
                    *offer_response = response;
                }
                Err(_) | Ok(_) => {
                    error!("Failed to get FW version for device {}", device_id);
                    return self.create_invalid_fw_version_response();
                }
            }
        }

        InternalResponseData::OfferResponse(
            self.customization
                .resolve_offer_response(&offer_responses[..self.devices.len()]),
        )
    }

    /// Process update content
    async fn process_give_content(&self, content: &FwUpdateContentCommand) -> InternalResponseData {
        let mut content_responses = [FwUpdateContentResponse::default(); MAX_SUPPORTED_DEVICES];

        if self.devices.len() > MAX_SUPPORTED_DEVICES {
            error!("More devices than supported");
            return Self::create_content_rejection(content.header.sequence_num);
        }

        for (device_id, content_response) in zip(self.devices.iter(), content_responses.iter_mut()) {
            let response = cfu::route_request(*device_id, RequestData::GiveContent(*content)).await;
            match response {
                Ok(InternalResponseData::ContentResponse(response)) => {
                    *content_response = response;
                }
                Err(_) | Ok(_) => {
                    error!("Failed to process content for device {}", device_id);
                    return Self::create_content_rejection(content.header.sequence_num);
                }
            }
        }

        InternalResponseData::ContentResponse(
            self.customization
                .resolve_content_response(&content_responses[..self.devices.len()]),
        )
    }

    /// Wait for a CFU message
    pub async fn wait_request(&self) -> RequestData {
        self.cfu_device.wait_request().await
    }

    /// Process a CFU message and produce a response
    pub async fn process_request(&self, request: RequestData) -> InternalResponseData {
        match request {
            RequestData::FwVersionRequest => {
                trace!("Got FwVersionRequest");
                self.process_get_fw_version().await
            }
            RequestData::GiveOffer(offer) => {
                trace!("Got GiveOffer");
                self.process_give_offer(&offer).await
            }
            RequestData::GiveContent(content) => {
                trace!("Got GiveContent");
                self.process_give_content(&content).await
            }
            RequestData::FinalizeUpdate => {
                trace!("Got FinalizeUpdate");
                InternalResponseData::ComponentPrepared
            }
            RequestData::PrepareComponentForUpdate => {
                trace!("Got PrepareComponentForUpdate");
                InternalResponseData::ComponentPrepared
            }
        }
    }

    /// Send a response to the CFU message
    pub async fn send_response(&self, response: InternalResponseData) {
        self.cfu_device.send_response(response).await;
    }

    pub async fn register(&'static self) -> Result<(), intrusive_list::Error> {
        cfu::register_device(&self.cfu_device).await
    }
}
