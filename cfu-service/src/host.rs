use core::future::Future;

use embedded_cfu_protocol::components::CfuComponentTraits;
use embedded_cfu_protocol::host::{CfuHostStates, CfuUpdater};
use embedded_cfu_protocol::protocol_definitions::*;
use embedded_cfu_protocol::{
    CfuImage,
    writer::{CfuWriterAsync, CfuWriterError, CfuWriterNop},
};
use heapless::Vec;

use crate::CfuError;

/// All host side Cfu traits, in some cases this will originate from a OS driver for CFU
pub trait CfuHost<W>: CfuHostStates<W> {
    /// Get all images
    fn get_cfu_images<I: CfuImage>(&self) -> impl Future<Output = Result<Vec<I, MAX_CMPT_COUNT>, CfuError>>;
    /// Gets the firmware version of all components
    fn get_all_fw_versions(
        self,
        writer: &mut W,
        primary_cmpt: ComponentId,
    ) -> impl Future<Output = Result<GetFwVersionResponse, CfuError>>;
    /// Goes through the offer list and returns a slice of offer responses
    fn process_cfu_offers<'a>(
        offer_commands: &'a [FwUpdateOffer],
        writer: &mut W,
    ) -> impl Future<Output = Result<&'a [FwUpdateOfferResponse], CfuError>>;
    /// For a specific component, update its content
    fn update_cfu_content(writer: &mut W) -> impl Future<Output = Result<FwUpdateContentResponse, CfuError>>;
    /// For a specific image that was updated, validate its content
    fn is_cfu_image_valid<I: CfuImage>(image: I) -> impl Future<Output = Result<bool, CfuError>>;
}

pub struct CfuHostInstance<I: CfuImage, C: CfuComponentTraits> {
    pub updater: CfuUpdater,
    pub images: heapless::Vec<I, MAX_CMPT_COUNT>,
    pub writer: CfuWriterNop,
    pub primary_cmpt: C,
    pub host_token: HostToken,
}

impl<I: CfuImage, C: CfuComponentTraits> CfuHostInstance<I, C> {
    #[allow(unused)]
    fn new(primary_cmpt: C) -> Self {
        Self {
            updater: CfuUpdater {},
            images: Vec::new(),
            writer: CfuWriterNop,
            primary_cmpt,
            host_token: HostToken::Driver,
        }
    }
}

impl<I: CfuImage, C: CfuComponentTraits, W: CfuWriterAsync> CfuHostStates<W> for CfuHostInstance<I, C> {
    async fn start_transaction(self, _writer: &mut W) -> Result<FwUpdateOfferResponse, CfuProtocolError> {
        let _mock_cmd = FwUpdateOfferInformation::new(OfferInformationComponentInfo::new(
            HostToken::Driver,
            SpecialComponentIds::Info,
            OfferInformationCodeValues::StartEntireTransaction,
        ));
        let mockresponse = FwUpdateOfferResponse::default();
        Ok(mockresponse)
    }
    async fn notify_start_offer_list(self, writer: &mut W) -> Result<FwUpdateOfferResponse, CfuProtocolError> {
        // Serialize FwUpdateOfferInformation to bytes
        let mock_cmd = FwUpdateOfferInformation::new(OfferInformationComponentInfo::new(
            HostToken::Driver,
            SpecialComponentIds::Info,
            OfferInformationCodeValues::StartOfferList,
        ));
        let serialized_mock: [u8; 16] = (&mock_cmd).into();

        let mut read = [0u8; 16];
        if let Ok(_result) = writer.cfu_write_read(None, &serialized_mock, &mut read).await {
            // Collect offer response
            let offer_response = FwUpdateOfferResponse::try_from(read)
                .map_err(|_| CfuProtocolError::WriterError(CfuWriterError::ByteConversionError))?;
            if offer_response.status != OfferStatus::Accept {
                Err(CfuProtocolError::CfuOfferStatusError(offer_response.status))
            } else {
                Ok(offer_response)
            }
        } else {
            Err(CfuProtocolError::WriterError(CfuWriterError::StorageError))
        }
    }

    async fn notify_end_offer_list(self, writer: &mut W) -> Result<FwUpdateOfferResponse, CfuProtocolError> {
        let mock_cmd = FwUpdateOfferInformation::new(OfferInformationComponentInfo::new(
            HostToken::Driver,
            SpecialComponentIds::Info,
            OfferInformationCodeValues::EndOfferList,
        ));
        let serialized_mock: [u8; 16] = (&mock_cmd).into();
        let mut read = [0u8; 16];
        if writer.cfu_write_read(None, &serialized_mock, &mut read).await.is_ok() {
            // Collect offer response
            let offer_response = FwUpdateOfferResponse::try_from(read)
                .map_err(|_| CfuProtocolError::WriterError(CfuWriterError::ByteConversionError))?;
            Ok(offer_response)
        } else {
            // unsuccessful write/read from the storage interface
            // use result.err() eventually
            Err(CfuProtocolError::WriterError(CfuWriterError::StorageError))
        }
    }

    async fn verify_all_updates_completed(resps: &[FwUpdateOfferResponse]) -> Result<bool, CfuProtocolError> {
        let mut bad_components: heapless::Vec<u8, MAX_CMPT_COUNT> = Vec::new();
        for (i, r) in resps.iter().enumerate() {
            if r.status != OfferStatus::Reject {
                let _ = bad_components.push(i as u8);
            }
        }
        if bad_components.is_empty() {
            Ok(true)
        } else {
            // probably want to have the component ids that didn't respond properly here too
            Err(CfuProtocolError::BadResponse)
        }
    }
}

impl<I: CfuImage, C: CfuComponentTraits, W: CfuWriterAsync> CfuHost<W> for CfuHostInstance<I, C> {
    async fn get_cfu_images<T: CfuImage>(&self) -> Result<Vec<T, MAX_CMPT_COUNT>, CfuError> {
        Err(CfuError::BadImage)
    }

    async fn get_all_fw_versions(
        self,
        _writer: &mut W,
        primary_cmpt: ComponentId,
    ) -> Result<GetFwVersionResponse, CfuError> {
        let mut component_count: u8 = 0;
        self.primary_cmpt.get_subcomponents().iter().for_each(|x| {
            if x.is_some() {
                component_count += 1
            }
        });
        let result = self.primary_cmpt.get_fw_version().await;
        if result.is_ok() {
            // convert bytes back to a GetFwVersionResponse
            let primary_component_version_info = FwVerComponentInfo::new(
                FwVersion {
                    major: 0,
                    minor: 1,
                    variant: 0,
                },
                primary_cmpt,
            );

            let mut component_info = [FwVerComponentInfo::default(); MAX_CMPT_COUNT]; // Create an array with 7 default elements
            component_info[0] = primary_component_version_info; // Set the first element to component_info

            let resp = GetFwVersionResponse {
                header: GetFwVersionResponseHeader::new(
                    1, // Component count
                    GetFwVerRespHeaderByte3::NoSpecialFlags,
                ),
                component_info,
            };
            Ok(resp)
        } else {
            Err(CfuError::ProtocolError(CfuProtocolError::BadResponse))
        }
    }

    async fn process_cfu_offers<'a>(
        _offer_commands: &'a [FwUpdateOffer],
        _writer: &mut W,
    ) -> Result<&'a [FwUpdateOfferResponse], CfuError> {
        // TODO
        Err(CfuError::BadImage)
    }

    async fn update_cfu_content(_writer: &mut W) -> Result<FwUpdateContentResponse, CfuError> {
        Err(CfuError::ProtocolError(CfuProtocolError::WriterError(
            CfuWriterError::Other,
        )))
    }

    async fn is_cfu_image_valid<T: CfuImage>(_image: T) -> Result<bool, CfuError> {
        Ok(true)
    }
}
