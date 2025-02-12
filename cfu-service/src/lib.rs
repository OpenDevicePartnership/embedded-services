#![no_std]
use core::future::Future;
use embedded_cfu_protocol::protocol_definitions::{
    CfuOfferStatus, CfuProtocolError, ComponentId, /*FwUpdateContentCommand,*/ FwUpdateContentResponse,
    FwUpdateOfferCommand, FwUpdateOfferResponse, FwVerComponentInfo, FwVersion, GetFwVersionCommand,
    GetFwVersionResponse, GetFwVersionResponseHeader, RejectReason,
};
use embedded_cfu_protocol::{CfuImage, CfuStates, CfuUpdater, CfuWriter, CfuWriterMock};
use heapless::Vec;

pub enum CfuError {
    BadImage,
    ProtocolError(CfuProtocolError),
}

// Max is (I think) 7 components in CfuUpdateOfferResponse
// There is probably a way to extend this, I need to read more
const MAX_CMPT_COUNT:usize = 7;

// use impl Future or Send so that these can be async?
pub trait Cfu<W, I>: CfuStates<W>
where
    W: CfuWriter,
    I: CfuImage,
{
    /// Get all images
    fn get_cfu_images(writer: W) -> impl Future<Output = Result<Vec<I, MAX_CMPT_COUNT>, CfuError>>;
    /// Gets the firmware version of all components
    fn get_all_fw_versions<T: CfuWriter, Tref: AsRef<[FwVerComponentInfo]>>(
        writer: T,
        primary_cmpt: ComponentId,
    ) -> impl Future<Output = Result<GetFwVersionResponse<Tref>, CfuError>>;
    /// Goes through the offer list and returns a slice of offer responses
    fn process_cfu_offers(
        offer_commands: &[FwUpdateOfferCommand],
        writer: W,
    ) -> impl Future<Output = Result<&[FwUpdateOfferResponse], CfuError>>;
    /// For a specific component, update its content
    fn update_cfu_content(writer: W) -> impl Future<Output = Result<FwUpdateContentResponse, CfuError>>;
    /// For a specific image, validate its content
    fn is_cfu_image_valid(image: I) -> impl Future<Output = Result<(), CfuError>>;
}

pub struct CfuMock<I: CfuImage> {
    updater: CfuUpdater,
    images: heapless::Vec<I, MAX_CMPT_COUNT>,
    writer: CfuWriterMock
}

impl <I: CfuImage> CfuMock<I> {
    fn new() -> Self {
        Self {
            updater: CfuUpdater {},
            images: Vec::new(),
            writer: CfuWriterMock::default(),
        }
    }
}

impl <W: CfuWriter, I: CfuImage> CfuStates<W> for CfuMock<I> {
    async fn start_transaction<T: CfuWriter>(host_token: u8, _writer: T) -> Result<FwUpdateOfferResponse, CfuProtocolError> {
        // Send GetFwVersion but seems like it's custom defined?
        let MockResponse = FwUpdateOfferResponse {
            token: host_token,
            rejectreasoncode: RejectReason::VendorSpecific(22),
            status: CfuOfferStatus::Accept,
        };
        Ok(MockResponse)
    }
    async fn notify_start_offer_list<T: CfuWriter>(_cmd: FwUpdateOfferCommand, writer: T) -> Result<FwUpdateOfferResponse, CfuProtocolError> {
        // Serialize FwUpdateOfferCommand to bytes, pull out componentid, host token
        let serialized_mock = [0u8;20];
        let cmpt_id_mock = 22;
        let ht_mock = 0;
        let mut read = [0u8;60];
        let _response = writer.write_read_to_component(cmpt_id_mock, &serialized_mock, &mut read);
        // convert back to FwUpdateOfferResponse
        Ok(FwUpdateOfferResponse {
            token: ht_mock,
            rejectreasoncode: RejectReason::VendorSpecific(22),
            status: CfuOfferStatus::Accept,
        })
    }

    async fn notify_end_offer_list<T: CfuWriter>(_cmd: FwUpdateOfferCommand, writer: T) -> Result<FwUpdateOfferResponse, CfuProtocolError> {
        // Serialize FwUpdateOfferCommand to bytes, pull out componentid, host token
        let serialized_mock = [0u8;20];
        let cmpt_id_mock = 22;
        let ht_mock = 0;
        let mut read = [0u8;60];
        let _response = writer.write_read_to_component(cmpt_id_mock, &serialized_mock, &mut read);
        // convert back to FwUpdateOfferResponse
        Ok(FwUpdateOfferResponse {
            token: ht_mock,
            rejectreasoncode: RejectReason::VendorSpecific(22),
            status: CfuOfferStatus::Command,
        })
    }

    async fn verify_all_updates_completed(resps: &[FwUpdateOfferResponse]) -> Result<bool, CfuProtocolError> {
        for r in resps.iter() {
            if r.status == CfuOfferStatus::Skip {
                return Ok(false)
            } else if r.status != CfuOfferStatus::Accept {
                return Err(CfuProtocolError::WriterError)
            }
        }
        Ok(true)
    }

}


impl<W: CfuWriter, I: CfuImage> Cfu<W,I> for CfuMock<I> {
    async fn get_cfu_images(_writer: W) -> Result<Vec<I, MAX_CMPT_COUNT>, CfuError> {
        Err(CfuError::BadImage)
    }

    async fn get_all_fw_versions<T: CfuWriter, Tref: AsRef<[FwVerComponentInfo]>>(writer: T, primary_cmpt: ComponentId) -> Result<GetFwVersionResponse<Tref>, CfuError> {
        let _cmd = GetFwVersionCommand {};
        // TODO: serialize command to bytes
        let cmd_bytes = [0u8;16];
        let mut resp_bytes = [0u8;16];
        // do this for all components, doing just 1 for this Mock
        let result = writer.write_read_to_component(primary_cmpt, &cmd_bytes, &mut resp_bytes).await;
        if result.is_ok() {
            // convert bytes back to a GetFwVersionResponse
            let blah_inner = FwVerComponentInfo {
                fw_version: FwVersion{
                    major: 0,
                    minor: 1,
                    variant: 0,
                },
                component_id: primary_cmpt,
                vendor_specific: 0,
                vendor_specific2: 0,
                bank: [true,true],
            };
            let resp: GetFwVersionResponse<Tref> = GetFwVersionResponse {
                header: GetFwVersionResponseHeader {
                    component_count: 1,
                    protocol_version: [false;4],
                    extensionflag: false,
                },
                component_info: &[blah_inner],
            };
            Ok(resp)
        } else {
            Err(CfuError::ProtocolError(CfuProtocolError::WriterError))
        }
    }

    async fn process_cfu_offers(
        _offer_commands: &[FwUpdateOfferCommand],
        _writer: W,
    ) -> Result<&[FwUpdateOfferResponse], CfuError> {
        // TODO
        Err(CfuError::BadImage)
    }

    async fn update_cfu_content(_writer: W) -> Result<FwUpdateContentResponse, CfuError>{
        Err(CfuError::ProtocolError(CfuProtocolError::WriterError))
    }

    async fn is_cfu_image_valid(_image: I) -> Result<bool, CfuError>{
        Ok(true)
    }
}
