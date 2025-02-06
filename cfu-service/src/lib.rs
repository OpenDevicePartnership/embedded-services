#![no_std]
use core::future::Future;
use embedded_cfu_protocol::protocol_definitions::{
    CfuProtocolError, ComponentId, FwUpdateContentResponse, FwUpdateOfferCommand, FwUpdateOfferResponse,
    FwVerComponentInfo, GetFwVersionResponse,
};
use embedded_cfu_protocol::{CfuImage, CfuStates, CfuWriter};
use heapless::Vec;

pub enum CfuError {
    BadImage,
    ProtocolError(CfuProtocolError),
}

// Max is (I think) 7 components in CfuUpdateOfferResponse
// There is probably a way to extend this, I need to read more
const MAX_CMPT_COUNT: usize = 7;
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
