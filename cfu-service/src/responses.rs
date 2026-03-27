use embedded_cfu_protocol::protocol_definitions::{
    CfuUpdateContentResponseStatus, ComponentId, FwUpdateContentResponse, FwVerComponentInfo, FwVersion,
    GetFwVerRespHeaderByte3, GetFwVersionResponse, GetFwVersionResponseHeader, MAX_CMPT_COUNT,
};

use embedded_services::cfu::component::InternalResponseData;

/// Returns a GetFwVersionResponse marked as invalid (version = 0xffffffff).
/// Used when a device fails to respond to a firmware version request.
pub(crate) fn create_invalid_fw_version_response(component_id: ComponentId) -> InternalResponseData {
    let dev_inf = FwVerComponentInfo::new(FwVersion::new(0xffffffff), component_id);
    let comp_info: [FwVerComponentInfo; MAX_CMPT_COUNT] = [dev_inf; MAX_CMPT_COUNT];
    InternalResponseData::FwVersionResponse(GetFwVersionResponse {
        header: GetFwVersionResponseHeader::new(1, GetFwVerRespHeaderByte3::NoSpecialFlags),
        component_info: comp_info,
    })
}

/// Returns a content rejection response with the given block sequence number.
/// Used when firmware content cannot be delivered or handled.
pub(crate) fn create_content_rejection(sequence: u16) -> InternalResponseData {
    InternalResponseData::ContentResponse(FwUpdateContentResponse::new(
        sequence,
        CfuUpdateContentResponseStatus::ErrorInvalid,
    ))
}
