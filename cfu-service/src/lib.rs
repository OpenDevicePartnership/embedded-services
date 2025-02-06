#![no_std]
use embedded_cfu_protocol::protocol_definitions::{
    CfuProtocolError, FwUpdateContentCommand, FwUpdateContentResponse, FwUpdateOfferCommand, FwUpdateOfferResponse,
};
use embedded_cfu_protocol::{CfuImage, CfuStates, CfuUpdateContent, CfuWriteError, CfuWriter};

pub enum CfuError {
    BadImage,
    WriteError(CfuWriteError),
    ProtocolError(CfuProtocolError),
}

// use impl Future or Send so that these can be async?
pub trait Cfu<W, I>: CfuUpdateContent<W> + CfuStates<W>
where
    W: CfuWriter,
    I: CfuImage,
{
    /// Goes through the offer list and returns a slice of offer responses
    fn process_cfu_offers(
        offer_commands: &[FwUpdateOfferCommand],
        writer: W,
    ) -> Result<&[FwUpdateOfferResponse], CfuError>;
    /// For a specific component, update its content
    fn update_cfu_content(
        content_command: FwUpdateContentCommand,
        writer: W,
    ) -> Result<FwUpdateContentResponse, CfuError>;
    /// For a specific image, validate its content
    fn validate_cfu_image(image: I) -> Result<(), CfuError>;
}
