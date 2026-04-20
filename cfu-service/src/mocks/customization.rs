//! Code related to mock for [`cfu_service::customization::Customization`]
extern crate std;

use std::collections::VecDeque;

use crate::customization::Customization;
use embedded_cfu_protocol::protocol_definitions::{
    FwUpdateOffer, FwUpdateOfferResponse, FwVersion, HostToken, OfferRejectReason, OfferStatus,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FnCall {
    Validate(FwVersion, FwUpdateOffer),
}

/// Simple mock
pub struct Mock {
    /// Queue to record function calls
    pub fn_calls: VecDeque<FnCall>,
    /// Acceptable version
    acceptable_version: FwVersion,
}

impl Mock {
    /// Create a new mock
    pub fn new(acceptable_version: FwVersion) -> Self {
        Self {
            fn_calls: VecDeque::new(),
            acceptable_version,
        }
    }

    fn record_fn_call(&mut self, fn_call: FnCall) {
        self.fn_calls.push_back(fn_call);
    }
}

impl Customization for Mock {
    fn validate(&mut self, current: FwVersion, fw_update_offer: &FwUpdateOffer) -> FwUpdateOfferResponse {
        self.record_fn_call(FnCall::Validate(current, *fw_update_offer));
        if fw_update_offer.firmware_version == self.acceptable_version {
            FwUpdateOfferResponse::new_accept(HostToken::Driver)
        } else {
            FwUpdateOfferResponse::new_with_failure(HostToken::Driver, OfferRejectReason::OldFw, OfferStatus::Reject)
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_validate_accept() {
        let acceptable_version = FwVersion::new(1);
        let mut mock = Mock::new(acceptable_version);
        let fw_update_offer = FwUpdateOffer::new(HostToken::Driver, 0, FwVersion::new(1), 0, 0);
        let response = mock.validate(acceptable_version, &fw_update_offer);
        assert_eq!(response, FwUpdateOfferResponse::new_accept(HostToken::Driver));
        assert_eq!(mock.fn_calls.len(), 1);
        assert_eq!(
            mock.fn_calls.pop_front(),
            Some(FnCall::Validate(acceptable_version, fw_update_offer))
        );
    }

    #[test]
    fn test_validate_reject() {
        let acceptable_version = FwVersion::new(1);
        let mut mock = Mock::new(acceptable_version);
        let fw_update_offer = FwUpdateOffer::new(HostToken::Driver, 0, FwVersion::new(9), 0, 0);
        let response = mock.validate(acceptable_version, &fw_update_offer);
        assert_eq!(
            response,
            FwUpdateOfferResponse::new_with_failure(HostToken::Driver, OfferRejectReason::OldFw, OfferStatus::Reject)
        );
        assert_eq!(mock.fn_calls.len(), 1);
        assert_eq!(
            mock.fn_calls.pop_front(),
            Some(FnCall::Validate(acceptable_version, fw_update_offer))
        );
    }
}
