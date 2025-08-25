//! Definitions for standard MPTF messages the generic Thermal service can expect
//!
//! Transport services such as eSPI and SSH would need to ensure messages are sent to the Thermal service in this format.
//!
//! This interface is subject to change as the eSPI OOB service is developed
use crate::mctp;
use crate::{self as ts, fan, sensor, utils};
use embassy_futures::select::Either;
use embedded_services::{comms, error};

/// MPTF Standard UUIDs which the thermal service understands
pub mod uuid_standard {
    pub const CRT_TEMP: uuid::Bytes = uuid::uuid!("218246e7-baf6-45f1-aa13-07e4845256b8").to_bytes_le();
    pub const PROC_HOT_TEMP: uuid::Bytes = uuid::uuid!("22dc52d2-fd0b-47ab-95b8-26552f9831a5").to_bytes_le();
    pub const PROFILE_TYPE: uuid::Bytes = uuid::uuid!("23b4a025-cdfd-4af9-a411-37a24c574615").to_bytes_le();
    pub const FAN_ON_TEMP: uuid::Bytes = uuid::uuid!("ba17b567-c368-48d5-bc6f-a312a41583c1").to_bytes_le();
    pub const FAN_RAMP_TEMP: uuid::Bytes = uuid::uuid!("3a62688c-d95b-4d2d-bacc-90d7a5816bcd").to_bytes_le();
    pub const FAN_MAX_TEMP: uuid::Bytes = uuid::uuid!("dcb758b1-f0fd-4ec7-b2c0-ef1e2a547b76").to_bytes_le();
    pub const FAN_MIN_RPM: uuid::Bytes = uuid::uuid!("db261c77-934b-45e2-9742-256c62badb7a").to_bytes_le();
    pub const FAN_MAX_RPM: uuid::Bytes = uuid::uuid!("5cf839df-8be7-42b9-9ac5-3403ca2c8a6a").to_bytes_le();
    pub const FAN_CURRENT_RPM: uuid::Bytes = uuid::uuid!("adf95492-0776-4ffc-84f3-b6c8b5269683").to_bytes_le();
}

/// Standard 32-bit DWORD
pub type Dword = u32;

/// 16-bit variable length
pub type VarLen = u16;

/// Instance ID
pub type InstanceId = u8;

/// Time in milliseconds
pub type Milliseconds = Dword;

/// MPTF expects temperatures in tenth Kelvins
pub type DeciKelvin = Dword;

/// MPTF Response
#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Response {
    // Status code (not necessarily related to Status code in response)
    // This is used because some commands can fail but don't contain Status output as part of MPTF spec
    pub status: Status,
    // Response data
    pub data: ResponseData,
}

impl Response {
    fn new(status: Status, data: ResponseData) -> Self {
        Self { status, data }
    }
}

/// MPTF Status
#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Status {
    /// Success
    Success,
    /// Invalid parameter was used
    InvalidParameter,
    /// Revision is not supported
    UnsupportedRevision,
    /// A hardware error occurred
    HardwareError,
}

impl From<Status> for u32 {
    fn from(status: Status) -> Self {
        match status {
            Status::Success => 0,
            Status::InvalidParameter => 1,
            Status::UnsupportedRevision => 2,
            Status::HardwareError => 3,
        }
    }
}

impl From<Status> for u8 {
    fn from(status: Status) -> Self {
        u32::from(status) as u8
    }
}

/// Standard MPTF requests expected by the thermal subsystem
#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Request {
    /// EC_THM_GET_TMP = 0x1
    GetTmp(InstanceId),
    /// EC_THM_SET_THRS = 0x2
    SetThrs(InstanceId, Milliseconds, DeciKelvin, DeciKelvin),
    /// EC_THM_GET_THRS = 0x3
    GetThrs(InstanceId),
    /// EC_THM_SET_SCP = 0x4
    SetScp(InstanceId, Dword, Dword, Dword),
    /// EC_THM_GET_VAR = 0x5
    GetVar(InstanceId, VarLen, uuid::Bytes),
    /// EC_THM_SET_VAR = 0x6
    SetVar(InstanceId, VarLen, uuid::Bytes, Dword),
}

impl From<Request> for u8 {
    fn from(request: Request) -> Self {
        match request {
            Request::GetTmp(_) => 1,
            Request::SetThrs(_, _, _, _) => 2,
            Request::GetThrs(_) => 3,
            Request::SetScp(_, _, _, _) => 4,
            Request::GetVar(_, _, _) => 5,
            Request::SetVar(_, _, _, _) => 6,
        }
    }
}

/// Data returned by thermal subsystem in response to MPTF requests  
#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum ResponseData {
    /// EC_THM_GET_TMP = 0x1
    GetTmp(DeciKelvin),
    /// EC_THM_SET_THRS = 0x2
    SetThrs(Status),
    /// EC_THM_GET_THRS = 0x3
    GetThrs(Status, Milliseconds, DeciKelvin, DeciKelvin),
    /// EC_THM_SET_SCP = 0x4
    SetScp(Status),
    /// EC_THM_GET_VAR = 0x5
    GetVar(Status, Dword),
    /// EC_THM_SET_VAR = 0x6
    SetVar(Status),
}

impl From<ResponseData> for u8 {
    fn from(response: ResponseData) -> Self {
        match response {
            ResponseData::GetTmp(_) => 1,
            ResponseData::SetThrs(_) => 2,
            ResponseData::GetThrs(_, _, _, _) => 3,
            ResponseData::SetScp(_) => 4,
            ResponseData::GetVar(_, _) => 5,
            ResponseData::SetVar(_) => 6,
        }
    }
}

/// Notifications to Host
#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Notify {
    /// Warn threshold was exceeded
    Warn,
    /// Prochot threshold was exceeded
    ProcHot,
    /// Critical threshold was exceeded
    Critical,
}

async fn sensor_get_tmp(tzid: InstanceId) -> Response {
    match ts::execute_sensor_request(sensor::DeviceId(tzid), sensor::Request::GetTemp).await {
        Ok(ts::sensor::ResponseData::Temp(temp)) => {
            Response::new(Status::Success, ResponseData::GetTmp(utils::c_to_dk(temp)))
        }
        _ => Response::new(Status::HardwareError, ResponseData::GetTmp(0)),
    }
}

async fn sensor_get_warn_thrs(tzid: InstanceId) -> Response {
    let low = ts::execute_sensor_request(
        sensor::DeviceId(tzid),
        sensor::Request::GetThreshold(sensor::ThresholdType::WarnLow),
    )
    .await;
    let high = ts::execute_sensor_request(
        sensor::DeviceId(tzid),
        sensor::Request::GetThreshold(sensor::ThresholdType::WarnHigh),
    )
    .await;

    match (low, high) {
        (Ok(sensor::ResponseData::Threshold(low)), Ok(sensor::ResponseData::Threshold(high))) => Response::new(
            Status::Success,
            ResponseData::GetThrs(Status::Success, 0, utils::c_to_dk(low), utils::c_to_dk(high)),
        ),
        _ => Response::new(Status::Success, ResponseData::GetThrs(Status::HardwareError, 0, 0, 0)),
    }
}

async fn sensor_set_warn_thrs(tzid: InstanceId, _timeout: Dword, low: Dword, high: Dword) -> Response {
    let low_res = ts::execute_sensor_request(
        sensor::DeviceId(tzid),
        sensor::Request::SetThreshold(sensor::ThresholdType::WarnLow, utils::dk_to_c(low)),
    )
    .await;
    let high_res = ts::execute_sensor_request(
        sensor::DeviceId(tzid),
        sensor::Request::SetThreshold(sensor::ThresholdType::WarnHigh, utils::dk_to_c(high)),
    )
    .await;

    if low_res.is_ok() && high_res.is_ok() {
        Response::new(Status::Success, ResponseData::SetThrs(Status::Success))
    } else {
        Response::new(Status::Success, ResponseData::SetThrs(Status::HardwareError))
    }
}

async fn sensor_get_thrs(instance: u8, threshold_type: sensor::ThresholdType) -> Response {
    match ts::execute_sensor_request(
        sensor::DeviceId(instance),
        sensor::Request::GetThreshold(threshold_type),
    )
    .await
    {
        Ok(sensor::ResponseData::Temp(temp)) => Response::new(
            Status::Success,
            ResponseData::GetVar(Status::Success, utils::c_to_dk(temp)),
        ),
        _ => Response::new(Status::Success, ResponseData::GetVar(Status::HardwareError, 0)),
    }
}

async fn fan_get_temp(instance: u8, fan_request: fan::Request) -> Response {
    match ts::execute_fan_request(fan::DeviceId(instance), fan_request).await {
        Ok(fan::ResponseData::Temp(temp)) => Response::new(
            Status::Success,
            ResponseData::GetVar(Status::Success, utils::c_to_dk(temp)),
        ),
        _ => Response::new(Status::Success, ResponseData::GetVar(Status::HardwareError, 0)),
    }
}

async fn fan_get_rpm(instance: u8, fan_request: fan::Request) -> Response {
    match ts::execute_fan_request(fan::DeviceId(instance), fan_request).await {
        Ok(fan::ResponseData::Rpm(rpm)) => {
            Response::new(Status::Success, ResponseData::GetVar(Status::Success, rpm as u32))
        }
        _ => Response::new(Status::Success, ResponseData::GetVar(Status::HardwareError, 0)),
    }
}

async fn sensor_set_thrs(instance: u8, threshold_type: sensor::ThresholdType, threshold_dk: Dword) -> Response {
    match ts::execute_sensor_request(
        sensor::DeviceId(instance),
        sensor::Request::SetThreshold(threshold_type, utils::dk_to_c(threshold_dk)),
    )
    .await
    {
        Ok(sensor::ResponseData::Success) => Response::new(Status::Success, ResponseData::SetVar(Status::Success)),
        _ => Response::new(Status::Success, ResponseData::SetVar(Status::HardwareError)),
    }
}

async fn fan_set_var(instance: u8, fan_request: fan::Request) -> Response {
    match ts::execute_fan_request(fan::DeviceId(instance), fan_request).await {
        Ok(fan::ResponseData::Success) => Response::new(Status::Success, ResponseData::SetVar(Status::Success)),
        _ => Response::new(Status::Success, ResponseData::SetVar(Status::HardwareError)),
    }
}

async fn process_request(request: Request) -> Response {
    match request {
        Request::GetTmp(tzid) => sensor_get_tmp(tzid).await,
        Request::GetThrs(tzid) => sensor_get_warn_thrs(tzid).await,
        Request::SetThrs(tzid, timeout, low, high) => sensor_set_warn_thrs(tzid, timeout, low, high).await,

        // TODO: How do we handle this genericly?
        Request::SetScp(_tzid, _policy_id, _acoustic_lim, _power_lim) => todo!(),

        Request::GetVar(instance, _len, uuid_standard::CRT_TEMP) => {
            sensor_get_thrs(instance, sensor::ThresholdType::Critical).await
        }
        Request::GetVar(instance, _len, uuid_standard::PROC_HOT_TEMP) => {
            sensor_get_thrs(instance, sensor::ThresholdType::Prochot).await
        }

        // TODO: Add a GetProfileId request type? But of sensor or fan?
        Request::GetVar(_instance, _len, uuid_standard::PROFILE_TYPE) => todo!(),

        Request::GetVar(instance, _len, uuid_standard::FAN_ON_TEMP) => {
            fan_get_temp(instance, fan::Request::GetOnTemp).await
        }
        Request::GetVar(instance, _len, uuid_standard::FAN_RAMP_TEMP) => {
            fan_get_temp(instance, fan::Request::GetRampTemp).await
        }
        Request::GetVar(instance, _len, uuid_standard::FAN_MAX_TEMP) => {
            fan_get_temp(instance, fan::Request::GetMaxTemp).await
        }
        Request::GetVar(instance, _len, uuid_standard::FAN_MIN_RPM) => {
            fan_get_rpm(instance, fan::Request::GetMinRpm).await
        }
        Request::GetVar(instance, _len, uuid_standard::FAN_MAX_RPM) => {
            fan_get_rpm(instance, fan::Request::GetMaxRpm).await
        }
        Request::GetVar(instance, _len, uuid_standard::FAN_CURRENT_RPM) => {
            fan_get_rpm(instance, fan::Request::GetRpm).await
        }

        Request::SetVar(instance, _len, uuid_standard::CRT_TEMP, temp_dk) => {
            sensor_set_thrs(instance, sensor::ThresholdType::Critical, temp_dk).await
        }
        Request::SetVar(instance, _len, uuid_standard::PROC_HOT_TEMP, temp_dk) => {
            sensor_set_thrs(instance, sensor::ThresholdType::Prochot, temp_dk).await
        }

        // TODO: Add a SetProfileId request type? But for sensor or fan?
        Request::SetVar(_instance, _len, uuid_standard::PROFILE_TYPE, _profile_id) => todo!(),

        Request::SetVar(instance, _len, uuid_standard::FAN_ON_TEMP, temp_dk) => {
            fan_set_var(instance, fan::Request::SetOnTemp(utils::dk_to_c(temp_dk))).await
        }
        Request::SetVar(instance, _len, uuid_standard::FAN_RAMP_TEMP, temp_dk) => {
            fan_set_var(instance, fan::Request::SetRampTemp(utils::dk_to_c(temp_dk))).await
        }
        Request::SetVar(instance, _len, uuid_standard::FAN_MAX_TEMP, temp_dk) => {
            fan_set_var(instance, fan::Request::SetMaxTemp(utils::dk_to_c(temp_dk))).await
        }

        // TODO: What does it mean to set the min/max RPM? Aren't these hardware defined?
        Request::SetVar(_instance, _len, uuid_standard::FAN_MIN_RPM, _rpm) => todo!(),
        Request::SetVar(_instance, _len, uuid_standard::FAN_MAX_RPM, _rpm) => todo!(),

        Request::SetVar(instance, _len, uuid_standard::FAN_CURRENT_RPM, rpm) => {
            fan_set_var(instance, fan::Request::SetRpm(rpm as u16)).await
        }

        // TODO: Allow OEM to handle these?
        Request::GetVar(_instance, _len, uuid) => {
            error!("Received GetVar for unrecognized UUID: {:?}", uuid);
            Response::new(
                Status::InvalidParameter,
                ResponseData::GetVar(Status::InvalidParameter, 0),
            )
        }
        Request::SetVar(_instance, _len, uuid, _value) => {
            error!("Received SetVar for unrecognized UUID: {:?}", uuid);
            Response::new(
                Status::InvalidParameter,
                ResponseData::GetVar(Status::InvalidParameter, 0),
            )
        }
    }
}

#[embassy_executor::task]
pub async fn handle_requests() {
    loop {
        let request = embassy_futures::select::select(ts::wait_mptf_request(), ts::wait_mctp_payload()).await;
        let send_result = match request {
            // Already in MPTF request format, handle as-is
            Either::First(request) => {
                let response = process_request(request).await;
                ts::send_service_msg(comms::EndpointID::External(comms::External::Host), &response).await
            }

            // A raw MCTP payload which we need to parse properly then encode the response packet back into
            Either::Second(mctp_payload) => {
                let request = Request::try_from(mctp_payload);
                let response = match request {
                    // Packet is OK
                    Ok(request) => {
                        let response = process_request(request).await;
                        mctp::AcpiMsgComms::from(response)
                    }
                    // Packet is malformed
                    Err(payload_error) => {
                        error!("Thermal received malformed packet");
                        mctp::AcpiMsgComms::from(payload_error)
                    }
                };
                ts::send_service_msg(
                    comms::EndpointID::External(comms::External::Host),
                    &embedded_services::ec_type::message::HostMsg::Response(response),
                )
                .await
            }
        };

        if send_result.is_err() {
            error!("Failed to send response to MPTF request!");
        }
    }
}
