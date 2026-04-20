//! Common CFU customization trait
use embedded_cfu_protocol::protocol_definitions::{FwUpdateOffer, FwUpdateOfferResponse, FwVersion};

/// Common CFU customization trait
pub trait Customization {
    /// Determine if we are accepting the firmware update offer, returns a CFU offer response
    fn validate(&mut self, current: FwVersion, offer: &FwUpdateOffer) -> FwUpdateOfferResponse;
}
