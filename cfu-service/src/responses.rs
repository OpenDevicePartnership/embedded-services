use embedded_cfu_protocol::protocol_definitions::{
    CfuUpdateContentResponseStatus, ComponentId, FwUpdateContentResponse, FwVerComponentInfo, FwVersion,
    GetFwVerRespHeaderByte3, GetFwVersionResponse, GetFwVersionResponseHeader, MAX_CMPT_COUNT,
};

use embedded_services::cfu::component::InternalResponseData;

const INVALID_FW_VERSION: u32 = u32::MAX;

// Returns a GetFwVersionResponse marked as invalid (version = INVALID_FW_VERSION).
// This is used when a device fails to respond to a firmware version request or responds with invalid/wrong data
pub(crate) fn create_invalid_fw_version_response(component_id: ComponentId) -> InternalResponseData {
    let dev_inf = FwVerComponentInfo::new(FwVersion::new(INVALID_FW_VERSION), component_id);
    let comp_info: [FwVerComponentInfo; MAX_CMPT_COUNT] = [dev_inf; MAX_CMPT_COUNT];
    InternalResponseData::FwVersionResponse(GetFwVersionResponse {
        header: GetFwVersionResponseHeader::new(1, GetFwVerRespHeaderByte3::NoSpecialFlags),
        component_info: comp_info,
    })
}

// Returns a content rejection response with the given block sequence number.
// This is used when firmware content cannot be delivered or handled correctly.
//
pub(crate) fn create_content_rejection(sequence: u16) -> InternalResponseData {
    InternalResponseData::ContentResponse(FwUpdateContentResponse::new(
        sequence,
        CfuUpdateContentResponseStatus::ErrorInvalid,
    ))
}
