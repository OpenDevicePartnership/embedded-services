//! Module that can broadcast CFU messages to multiple devices
//! This allows devices to share a single component ID

use core::{future::Future, iter::zip};

use embassy_futures::join::{join, join3, join4};
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
    fn resolve_fw_versions(&self, versions: &[GetFwVersionResponse]) -> GetFwVersionResponse;

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
pub const MAX_SUPPORTED_DEVICES: usize = 4;

impl<'a, C: Customization> Splitter<'a, C> {
    /// Create a new Splitter, returns None if the devices slice is empty or too large
    pub fn new(component_id: ComponentId, devices: &'a [ComponentId], customization: C) -> Option<Self> {
        if devices.is_empty() || devices.len() > MAX_SUPPORTED_DEVICES {
            None
        } else {
            Some(Self {
                cfu_device: CfuDevice::new(component_id),
                devices,
                customization,
            })
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

    /// Create a content rejection response
    fn create_content_rejection(sequence: u16) -> InternalResponseData {
        InternalResponseData::ContentResponse(FwUpdateContentResponse::new(
            sequence,
            CfuUpdateContentResponseStatus::ErrorInvalid,
        ))
    }

    /// Process a fw version request
    async fn process_get_fw_version(&self) -> InternalResponseData {
        let mut versions = [GetFwVersionResponse {
            header: Default::default(),
            component_info: Default::default(),
        }; MAX_SUPPORTED_DEVICES];

        let success = map_slice_join(self.devices, &mut versions, |device_id| async move {
            if let Ok(InternalResponseData::FwVersionResponse(version_info)) =
                cfu::route_request(*device_id, RequestData::FwVersionRequest).await
            {
                Some(version_info)
            } else {
                error!("Failed to get FW version for device {}", device_id);
                None
            }
        })
        .await;

        if !success {
            self.create_invalid_fw_version_response()
        } else if let Some(versions) = versions.get(..self.devices.len()) {
            let mut overall_version = self.customization.resolve_fw_versions(versions);

            // The overall component version comes first
            overall_version.component_info[0].component_id = self.cfu_device.component_id();
            InternalResponseData::FwVersionResponse(overall_version)
        } else {
            self.create_invalid_fw_version_response()
        }
    }

    /// Process a give offer request
    async fn process_give_offer(&self, offer: &FwUpdateOffer) -> InternalResponseData {
        let mut offer_responses = [FwUpdateOfferResponse::default(); MAX_SUPPORTED_DEVICES];

        let success = map_slice_join(self.devices, &mut offer_responses, |device_id| async move {
            let mut offer = *offer;

            // Override with the correct component ID for the device
            offer.component_info.component_id = *device_id;
            if let Ok(InternalResponseData::OfferResponse(response)) =
                cfu::route_request(*device_id, RequestData::GiveOffer(offer)).await
            {
                Some(response)
            } else {
                error!("Failed to get FW version for device {}", device_id);
                None
            }
        })
        .await;

        if success && let Some(offer_responses_slice) = offer_responses.get(..self.devices.len()) {
            InternalResponseData::OfferResponse(self.customization.resolve_offer_response(offer_responses_slice))
        } else {
            self.create_invalid_fw_version_response()
        }
    }

    /// Process update content
    async fn process_give_content(&self, content: &FwUpdateContentCommand) -> InternalResponseData {
        let mut content_responses = [FwUpdateContentResponse::default(); MAX_SUPPORTED_DEVICES];

        let success = map_slice_join(self.devices, &mut content_responses, |device_id| async move {
            if let Ok(InternalResponseData::ContentResponse(response)) =
                cfu::route_request(*device_id, RequestData::GiveContent(*content)).await
            {
                Some(response)
            } else {
                error!("Failed to get FW version for device {}", device_id);
                None
            }
        })
        .await;

        if success && let Some(content_responses_slice) = content_responses.get(..self.devices.len()) {
            InternalResponseData::ContentResponse(self.customization.resolve_content_response(content_responses_slice))
        } else {
            Self::create_content_rejection(content.header.sequence_num)
        }
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
            RequestData::AbortUpdate => {
                trace!("Got AbortUpdate");
                InternalResponseData::ComponentPrepared
            }
            RequestData::FinalizeUpdate => {
                trace!("Got FinalizeUpdate");
                InternalResponseData::ComponentPrepared
            }
            RequestData::PrepareComponentForUpdate => {
                trace!("Got PrepareComponentForUpdate");
                InternalResponseData::ComponentPrepared
            }
            RequestData::GiveOfferExtended(_) => {
                trace!("Got GiveExtendedOffer");
                // Extended offers are not currently supported
                InternalResponseData::OfferResponse(FwUpdateOfferResponse::new_with_failure(
                    HostToken::Driver,
                    OfferRejectReason::InvalidComponent,
                    OfferStatus::Reject,
                ))
            }
            RequestData::GiveOfferInformation(_) => {
                trace!("Got GiveOfferInformation");
                // Offer information is not currently supported
                InternalResponseData::OfferResponse(FwUpdateOfferResponse::new_with_failure(
                    HostToken::Driver,
                    OfferRejectReason::InvalidComponent,
                    OfferStatus::Reject,
                ))
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

/// Map items in an input slice to an output slice using an async closure.
///
/// This function will execute the closure concurrently in groups up to four items at a time.
/// Four is an arbitrary but is a balance between two (easy to implement, but not very concurrent) and eight (more implementation work).
/// This will exit early and return false if any item results in `None`.
async fn map_slice_join<'i, 'o, I, O, F: Future<Output = Option<O>>>(
    input: &'i [I],
    output: &'o mut [O],
    f: impl Fn(&'i I) -> F,
) -> bool {
    let mut iter = zip(input.iter(), output.iter_mut());
    loop {
        // panic safety: other combinations aren't possible because we're using a fused iterator
        #[allow(clippy::unreachable)]
        match (iter.next(), iter.next(), iter.next(), iter.next()) {
            (None, None, None, None) => {
                // No more items to process
                return true;
            }
            (Some((i0, o0)), None, None, None) => {
                if let Some(result) = f(i0).await {
                    *o0 = result;
                } else {
                    return false;
                }
            }
            (Some((i0, o0)), Some((i1, o1)), None, None) => {
                let results = join(f(i0), f(i1)).await;
                if let (Some(r0), Some(r1)) = results {
                    *o0 = r0;
                    *o1 = r1;
                } else {
                    return false;
                }
            }
            (Some((i0, o0)), Some((i1, o1)), Some((i2, o2)), None) => {
                let results = join3(f(i0), f(i1), f(i2)).await;
                if let (Some(r0), Some(r1), Some(r2)) = results {
                    *o0 = r0;
                    *o1 = r1;
                    *o2 = r2;
                } else {
                    return false;
                }
            }
            (Some((i0, o0)), Some((i1, o1)), Some((i2, o2)), Some((i3, o3))) => {
                let results = join4(f(i0), f(i1), f(i2), f(i3)).await;
                if let (Some(r0), Some(r1), Some(r2), Some(r3)) = results {
                    *o0 = r0;
                    *o1 = r1;
                    *o2 = r2;
                    *o3 = r3;
                } else {
                    return false;
                }
            }
            _ => {
                unreachable!()
            }
        }
    }
}
