use binary_serde::{BinarySerde, Endianness};
use core::future::Future;
use embedded_cfu_protocol::components::CfuComponentTraits;
use embedded_cfu_protocol::protocol_definitions::*;
use embedded_cfu_protocol::{CfuHostStates, CfuImage, CfuUpdater};
use embedded_cfu_protocol::{CfuWriter, CfuWriterDefault, CfuWriterError};
use heapless::Vec;

use crate::CfuError;

/// All host side Cfu traits, in some cases this will originate from a OS driver for CFU
pub trait CfuHost: CfuHostStates {
    /// Get all images
    fn get_cfu_images<I: CfuImage>(&self) -> impl Future<Output = Result<Vec<I, MAX_CMPT_COUNT>, CfuError>>;
    /// Gets the firmware version of all components
    fn get_all_fw_versions<T: CfuWriter>(
        self,
        writer: &mut T,
        primary_cmpt: ComponentId,
    ) -> impl Future<Output = Result<GetFwVersionResponse, CfuError>>;
    /// Goes through the offer list and returns a slice of offer responses
    fn process_cfu_offers<'a, T: CfuWriter>(
        offer_commands: &'a [FwUpdateOfferCommand],
        writer: &mut T,
    ) -> impl Future<Output = Result<&'a [FwUpdateOfferResponse], CfuError>>;
    /// For a specific component, update its content
    fn update_cfu_content<T: CfuWriter>(
        writer: &mut T,
    ) -> impl Future<Output = Result<FwUpdateContentResponse, CfuError>>;
    /// For a specific image that was updated, validate its content
    fn is_cfu_image_valid<I: CfuImage>(image: I) -> impl Future<Output = Result<bool, CfuError>>;
}

pub struct CfuHostInstance<I: CfuImage, C: CfuComponentTraits> {
    pub updater: CfuUpdater,
    pub images: heapless::Vec<I, MAX_CMPT_COUNT>,
    pub writer: CfuWriterDefault,
    pub primary_cmpt: C,
    pub host_token: HostToken,
}

impl<I: CfuImage, C: CfuComponentTraits> CfuHostInstance<I, C> {
    #[allow(unused)]
    fn new(primary_cmpt: C) -> Self {
        Self {
            updater: CfuUpdater {},
            images: Vec::new(),
            writer: CfuWriterDefault::default(),
            primary_cmpt,
            host_token: 0,
        }
    }
}

impl<I: CfuImage, C: CfuComponentTraits> CfuHostStates for CfuHostInstance<I, C> {
    async fn start_transaction<T: CfuWriter>(self, _writer: &mut T) -> Result<FwUpdateOfferResponse, CfuProtocolError> {
        let component_id = self.primary_cmpt.get_component_id();
        let _mock_cmd = FwUpdateOfferCommand::new_with_command(
            self.host_token,
            component_id,
            FwVersion::default(),
            InformationCodeValues::StartOfferList,
        );
        let mockresponse = FwUpdateOfferResponse::default();
        Ok(mockresponse)
    }
    async fn notify_start_offer_list<T: CfuWriter>(
        self,
        writer: &mut T,
    ) -> Result<FwUpdateOfferResponse, CfuProtocolError> {
        // Serialize FwUpdateOfferCommand to bytes, pull out componentid, host token
        let component_id = self.primary_cmpt.get_component_id();
        let mock_cmd = FwUpdateOfferCommand::new_with_command(
            self.host_token,
            component_id,
            FwVersion::default(),
            InformationCodeValues::StartOfferList,
        );
        let mut serialized_mock = [0u8; 20];
        FwUpdateOfferCommand::binary_serialize(&mock_cmd, &mut serialized_mock, Endianness::Little);
        let mut read = [0u8; 60];
        //self.primary_cmpt.me.writer.write_read_to_component(Some(component_id), &serialized_mock, &mut read).await;
        if let Ok(_result) = writer.cfu_write_read(None, &serialized_mock, &mut read).await {
            if let Ok(converted) = FwUpdateOfferResponse::binary_deserialize(&read, Endianness::Little) {
                if converted.status != CfuOfferResponseStatus::Success {
                    Err(CfuProtocolError::CfuResponseError(converted.status))
                } else {
                    Ok(converted)
                }
            } else {
                Err(CfuProtocolError::WriterError(CfuWriterError::ByteConversionError))
            }
        } else {
            Err(CfuProtocolError::WriterError(CfuWriterError::StorageError))
        }
    }

    async fn notify_end_offer_list<T: CfuWriter>(
        self,
        writer: &mut T,
    ) -> Result<FwUpdateOfferResponse, CfuProtocolError> {
        let component_id = self.primary_cmpt.get_component_id();
        let mock_cmd = FwUpdateOfferCommand::new_with_command(
            self.host_token,
            component_id,
            FwVersion::default(),
            InformationCodeValues::EndOfferList,
        );
        let mut serialized_mock = [0u8; 20]; // TODO CHECK SIZE
        FwUpdateOfferCommand::binary_serialize(&mock_cmd, &mut serialized_mock, Endianness::Little);
        let mut read = [0u8; 60];
        if let Ok(_response) = writer.cfu_write_read(None, &serialized_mock, &mut read).await {
            // convert back to FwUpdateOfferResponse
            if let Ok(converted) = FwUpdateOfferResponse::binary_deserialize(&read, Endianness::Little) {
                Ok(converted)
            } else {
                // error deserializing the bytes that were read
                Err(CfuProtocolError::WriterError(CfuWriterError::ByteConversionError))
            }
        } else {
            // unsuccessful write/read from the storage interface
            // use result.err() eventually
            Err(CfuProtocolError::WriterError(CfuWriterError::StorageError))
        }
    }

    async fn verify_all_updates_completed(resps: &[FwUpdateOfferResponse]) -> Result<bool, CfuProtocolError> {
        let mut bad_components: heapless::Vec<u8, MAX_CMPT_COUNT> = Vec::new();
        for (i, r) in resps.iter().enumerate() {
            if r.status != CfuOfferResponseStatus::Success {
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

impl<I: CfuImage, C: CfuComponentTraits> CfuHost for CfuHostInstance<I, C> {
    async fn get_cfu_images<T: CfuImage>(&self) -> Result<Vec<T, MAX_CMPT_COUNT>, CfuError> {
        Err(CfuError::BadImage)
    }

    async fn get_all_fw_versions<T: CfuWriter>(
        self,
        _writer: &mut T,
        primary_cmpt: ComponentId,
    ) -> Result<GetFwVersionResponse, CfuError> {
        let mut vec: Vec<FwVerComponentInfo, MAX_CMPT_COUNT> = Vec::new();
        let mut component_count: u8 = 0;
        if let Some(subs) = self.primary_cmpt.get_subcomponents() {
            component_count = subs.len() as u8
        }
        let result = self.primary_cmpt.get_fw_version().await;
        if result.is_ok() {
            // convert bytes back to a GetFwVersionResponse
            let inner = FwVerComponentInfo::new(
                FwVersion {
                    major: 0,
                    minor: 1,
                    variant: 0,
                },
                primary_cmpt,
                BankType::DualBank,
            );
            let _ = vec.push(inner);
            let arr = vec.into_array().unwrap();
            let resp: GetFwVersionResponse = GetFwVersionResponse {
                header: GetFwVersionResponseHeader::new(component_count, GetFwVerRespHeaderByte0::default()),
                component_info: arr,
            };
            Ok(resp)
        } else {
            Err(CfuError::ProtocolError(CfuProtocolError::BadResponse))
        }
    }

    async fn process_cfu_offers<'a, T: CfuWriter>(
        _offer_commands: &'a [FwUpdateOfferCommand],
        _writer: &mut T,
    ) -> Result<&'a [FwUpdateOfferResponse], CfuError> {
        // TODO
        Err(CfuError::BadImage)
    }

    async fn update_cfu_content<T: CfuWriter>(_writer: &mut T) -> Result<FwUpdateContentResponse, CfuError> {
        Err(CfuError::ProtocolError(CfuProtocolError::WriterError(
            CfuWriterError::Other,
        )))
    }

    async fn is_cfu_image_valid<T: CfuImage>(_image: T) -> Result<bool, CfuError> {
        Ok(true)
    }
}
