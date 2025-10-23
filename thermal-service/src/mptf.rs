//! Definitions for standard MPTF messages the generic Thermal service can expect
//!
//! Transport services such as eSPI and SSH would need to ensure messages are sent to the Thermal service in this format.
//!
//! This interface is subject to change as the eSPI OOB service is developed
use crate::{self as ts, fan, sensor, utils};
use embedded_services::ec_type::message::{StdHostPayload, StdHostRequest};
use embedded_services::{comms, ec_type::protocols::mctp, error};

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

async fn sensor_get_tmp(request: &mut StdHostRequest) {
    match request.payload {
        mctp::Odp::ThermalGetTmpRequest { instance_id } => {
            match ts::execute_sensor_request(sensor::DeviceId(instance_id), sensor::Request::GetTemp).await {
                Ok(ts::sensor::ResponseData::Temp(temp)) => {
                    request.payload = StdHostPayload::ThermalGetTmpResponse {
                        temperature: utils::c_to_dk(temp),
                    };
                    request.status = 0;
                }
                _ => {
                    request.payload = StdHostPayload::ErrorResponse {};
                    request.status = 1;
                }
            }
        }
        _ => error!("Thermal Service: Host message header and payload mismatch"),
    }
}

async fn get_var_handler(request: &mut StdHostRequest) {
    match request.payload {
        mctp::Odp::ThermalGetVarRequest {
            instance_id,
            len: _,
            var_uuid,
        } => match var_uuid {
            uuid_standard::CRT_TEMP => {
                let Response { status: _, data } = sensor_get_thrs(instance_id, sensor::ThresholdType::Critical).await;
                if let ResponseData::GetVar(Status::Success, val) = data {
                    request.status = Status::Success.into();
                    request.payload = mctp::Odp::ThermalGetVarResponse {
                        status: Status::Success.into(),
                        val,
                    }
                } else if let ResponseData::GetVar(error, val) = data {
                    request.payload = mctp::Odp::ThermalGetVarResponse {
                        status: error.into(),
                        val,
                    }
                }
            }
            uuid_standard::PROC_HOT_TEMP => {
                let Response { status: _, data } = sensor_get_thrs(instance_id, sensor::ThresholdType::Prochot).await;
                if let ResponseData::GetVar(Status::Success, val) = data {
                    request.status = Status::Success.into();
                    request.payload = mctp::Odp::ThermalGetVarResponse {
                        status: Status::Success.into(),
                        val,
                    }
                } else if let ResponseData::GetVar(error, val) = data {
                    request.payload = mctp::Odp::ThermalGetVarResponse {
                        status: error.into(),
                        val,
                    }
                }
            }
            // TODO: Add a SetProfileId request type? But for sensor or fan?
            uuid_standard::PROFILE_TYPE => {
                todo!()
            }
            uuid_standard::FAN_ON_TEMP => {
                let Response { status: _, data } = fan_get_temp(instance_id, fan::Request::GetOnTemp).await;
                if let ResponseData::GetVar(Status::Success, val) = data {
                    request.status = Status::Success.into();
                    request.payload = mctp::Odp::ThermalGetVarResponse {
                        status: Status::Success.into(),
                        val,
                    }
                } else if let ResponseData::GetVar(error, val) = data {
                    request.payload = mctp::Odp::ThermalGetVarResponse {
                        status: error.into(),
                        val,
                    }
                }
            }
            uuid_standard::FAN_RAMP_TEMP => {
                let Response { status: _, data } = fan_get_temp(instance_id, fan::Request::GetRampTemp).await;
                if let ResponseData::GetVar(Status::Success, val) = data {
                    request.status = Status::Success.into();
                    request.payload = mctp::Odp::ThermalGetVarResponse {
                        status: Status::Success.into(),
                        val,
                    }
                } else if let ResponseData::GetVar(error, val) = data {
                    request.payload = mctp::Odp::ThermalGetVarResponse {
                        status: error.into(),
                        val,
                    }
                }
            }
            uuid_standard::FAN_MAX_TEMP => {
                let Response { status: _, data } = fan_get_temp(instance_id, fan::Request::GetMaxTemp).await;
                if let ResponseData::GetVar(Status::Success, val) = data {
                    request.status = Status::Success.into();
                    request.payload = mctp::Odp::ThermalGetVarResponse {
                        status: Status::Success.into(),
                        val,
                    }
                } else if let ResponseData::GetVar(error, val) = data {
                    request.payload = mctp::Odp::ThermalGetVarResponse {
                        status: error.into(),
                        val,
                    }
                }
            }
            uuid_standard::FAN_MIN_RPM => {
                let Response { status: _, data } = fan_get_rpm(instance_id, fan::Request::GetMinRpm).await;
                if let ResponseData::GetVar(Status::Success, val) = data {
                    request.status = Status::Success.into();
                    request.payload = mctp::Odp::ThermalGetVarResponse {
                        status: Status::Success.into(),
                        val,
                    }
                } else if let ResponseData::GetVar(error, val) = data {
                    request.payload = mctp::Odp::ThermalGetVarResponse {
                        status: error.into(),
                        val,
                    }
                }
            }
            uuid_standard::FAN_MAX_RPM => {
                let Response { status: _, data } = fan_get_rpm(instance_id, fan::Request::GetMaxRpm).await;
                if let ResponseData::GetVar(Status::Success, val) = data {
                    request.status = Status::Success.into();
                    request.payload = mctp::Odp::ThermalGetVarResponse {
                        status: Status::Success.into(),
                        val,
                    }
                } else if let ResponseData::GetVar(error, val) = data {
                    request.status = error.into();
                    request.payload = mctp::Odp::ThermalGetVarResponse {
                        status: error.into(),
                        val,
                    }
                }
            }
            uuid_standard::FAN_CURRENT_RPM => {
                let Response { status: _, data } = fan_get_rpm(instance_id, fan::Request::GetRpm).await;
                if let ResponseData::GetVar(Status::Success, val) = data {
                    request.payload = mctp::Odp::ThermalGetVarResponse {
                        status: Status::Success.into(),
                        val,
                    }
                } else if let ResponseData::GetVar(error, val) = data {
                    request.status = error.into();
                    request.payload = mctp::Odp::ThermalGetVarResponse {
                        status: error.into(),
                        val,
                    }
                }
            }
            // TODO: Allow OEM to handle these?
            uuid => {
                error!("Received GetVar for unrecognized UUID: {:?}", uuid);
                request.status = Status::InvalidParameter.into();
                request.payload = mctp::Odp::ThermalGetVarResponse {
                    status: Status::InvalidParameter.into(),
                    val: 0,
                }
            }
        },
        _ => error!("Thermal Service: Host message header and payload mismatch"),
    }
}

async fn set_var_handler(request: &mut StdHostRequest) {
    match request.payload {
        mctp::Odp::ThermalSetVarRequest {
            instance_id,
            len: _,
            var_uuid,
            set_var,
        } => match var_uuid {
            uuid_standard::CRT_TEMP => {
                let Response { status: _, data } =
                    sensor_set_thrs(instance_id, sensor::ThresholdType::Critical, set_var).await;
                if let ResponseData::SetVar(Status::Success) = data {
                    request.status = Status::Success.into();
                    request.payload = mctp::Odp::ThermalSetVarResponse {
                        status: Status::Success.into(),
                    }
                } else if let ResponseData::SetVar(error) = data {
                    request.payload = mctp::Odp::ThermalSetVarResponse { status: error.into() }
                }
            }
            uuid_standard::PROC_HOT_TEMP => {
                let Response { status: _, data } =
                    sensor_set_thrs(instance_id, sensor::ThresholdType::Prochot, set_var).await;
                if let ResponseData::SetVar(Status::Success) = data {
                    request.status = Status::Success.into();
                    request.payload = mctp::Odp::ThermalSetVarResponse {
                        status: Status::Success.into(),
                    }
                } else if let ResponseData::SetVar(error) = data {
                    request.payload = mctp::Odp::ThermalSetVarResponse { status: error.into() }
                }
            }
            // TODO: Add a SetProfileId request type? But for sensor or fan?
            uuid_standard::PROFILE_TYPE => {
                todo!()
            }
            uuid_standard::FAN_ON_TEMP => {
                let Response { status: _, data } =
                    fan_set_var(instance_id, fan::Request::SetOnTemp(utils::dk_to_c(set_var))).await;
                if let ResponseData::SetVar(Status::Success) = data {
                    request.status = Status::Success.into();
                    request.payload = mctp::Odp::ThermalSetVarResponse {
                        status: Status::Success.into(),
                    }
                } else if let ResponseData::SetVar(error) = data {
                    request.payload = mctp::Odp::ThermalSetVarResponse { status: error.into() }
                }
            }
            uuid_standard::FAN_RAMP_TEMP => {
                let Response { status: _, data } =
                    fan_set_var(instance_id, fan::Request::SetRampTemp(utils::dk_to_c(set_var))).await;
                if let ResponseData::SetVar(Status::Success) = data {
                    request.status = Status::Success.into();
                    request.payload = mctp::Odp::ThermalSetVarResponse {
                        status: Status::Success.into(),
                    }
                } else if let ResponseData::SetVar(error) = data {
                    request.payload = mctp::Odp::ThermalSetVarResponse { status: error.into() }
                }
            }
            uuid_standard::FAN_MAX_TEMP => {
                let Response { status: _, data } =
                    fan_set_var(instance_id, fan::Request::SetMaxTemp(utils::dk_to_c(set_var))).await;
                if let ResponseData::SetVar(Status::Success) = data {
                    request.status = Status::Success.into();
                    request.payload = mctp::Odp::ThermalSetVarResponse {
                        status: Status::Success.into(),
                    }
                } else if let ResponseData::SetVar(error) = data {
                    request.payload = mctp::Odp::ThermalSetVarResponse { status: error.into() }
                }
            }
            // TODO: What does it mean to set the min/max RPM? Aren't these hardware defined?
            uuid_standard::FAN_MIN_RPM => {
                todo!()
            }
            // TODO: What does it mean to set the min/max RPM? Aren't these hardware defined?
            uuid_standard::FAN_MAX_RPM => {
                todo!()
            }
            uuid_standard::FAN_CURRENT_RPM => {
                let Response { status: _, data } = fan_set_var(instance_id, fan::Request::SetRpm(set_var as u16)).await;
                if let ResponseData::SetVar(Status::Success) = data {
                    request.payload = mctp::Odp::ThermalSetVarResponse {
                        status: Status::Success.into(),
                    }
                } else if let ResponseData::SetVar(error) = data {
                    request.status = error.into();
                    request.payload = mctp::Odp::ThermalSetVarResponse { status: error.into() }
                }
            }
            // TODO: Allow OEM to handle these?
            uuid => {
                error!("Received SetVar for unrecognized UUID: {:?}", uuid);
                request.status = Status::InvalidParameter.into();
                request.payload = mctp::Odp::ThermalSetVarResponse {
                    status: Status::InvalidParameter.into(),
                }
            }
        },
        _ => error!("Thermal Service: Host message header and payload mismatch"),
    }
}

async fn sensor_get_warn_thrs(request: &mut StdHostRequest) {
    match request.payload {
        mctp::Odp::ThermalGetThrsRequest { instance_id } => {
            let low = ts::execute_sensor_request(
                sensor::DeviceId(instance_id),
                sensor::Request::GetThreshold(sensor::ThresholdType::WarnLow),
            )
            .await;
            let high = ts::execute_sensor_request(
                sensor::DeviceId(instance_id),
                sensor::Request::GetThreshold(sensor::ThresholdType::WarnHigh),
            )
            .await;

            match (low, high) {
                (Ok(sensor::ResponseData::Threshold(low)), Ok(sensor::ResponseData::Threshold(high))) => {
                    request.payload = StdHostPayload::ThermalGetThrsResponse {
                        status: 0,
                        timeout: 0,
                        low: utils::c_to_dk(low),
                        high: utils::c_to_dk(high),
                    };
                    request.status = 0;
                }
                _ => {
                    request.payload = StdHostPayload::ThermalGetThrsResponse {
                        status: 1,
                        timeout: 0,
                        low: 0,
                        high: 0,
                    };
                    request.status = 1;
                }
            }
        }
        _ => error!("Thermal Service: Host message header and payload mismatch"),
    }
}

async fn sensor_set_warn_thrs(request: &mut StdHostRequest) {
    match request.payload {
        mctp::Odp::ThermalSetThrsRequest {
            instance_id,
            timeout: _,
            low,
            high,
        } => {
            let low_res = ts::execute_sensor_request(
                sensor::DeviceId(instance_id),
                sensor::Request::SetThreshold(sensor::ThresholdType::WarnLow, utils::dk_to_c(low)),
            )
            .await;
            let high_res = ts::execute_sensor_request(
                sensor::DeviceId(instance_id),
                sensor::Request::SetThreshold(sensor::ThresholdType::WarnHigh, utils::dk_to_c(high)),
            )
            .await;

            if low_res.is_ok() && high_res.is_ok() {
                request.payload = mctp::Odp::ThermalSetThrsResponse { status: 0 };
                request.status = 0;
            } else {
                request.payload = mctp::Odp::ThermalSetThrsResponse { status: 1 };
                request.status = 1;
            }
        }
        _ => error!("Thermal Service: Host message header and payload mismatch"),
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

async fn process_request(request: &mut StdHostRequest) {
    match request.command {
        embedded_services::ec_type::message::OdpCommand::Thermal(thermal_msg) => match thermal_msg {
            embedded_services::ec_type::protocols::mptf::ThermalCmd::GetTmp => sensor_get_tmp(request).await,
            embedded_services::ec_type::protocols::mptf::ThermalCmd::SetThrs => sensor_set_warn_thrs(request).await,
            embedded_services::ec_type::protocols::mptf::ThermalCmd::GetThrs => sensor_get_warn_thrs(request).await,
            // TODO: How do we handle this genericly?
            embedded_services::ec_type::protocols::mptf::ThermalCmd::SetScp => todo!(),
            embedded_services::ec_type::protocols::mptf::ThermalCmd::GetVar => get_var_handler(request).await,
            embedded_services::ec_type::protocols::mptf::ThermalCmd::SetVar => set_var_handler(request).await,
        },
        _ => error!("Thermal Service: Recvd other subsystem host message"),
    }
}

#[embassy_executor::task]
pub async fn handle_requests() {
    loop {
        let mut request = ts::wait_mctp_payload().await;
        process_request(&mut request).await;
        let send_result = ts::send_service_msg(
            comms::EndpointID::External(comms::External::Host),
            &embedded_services::ec_type::message::HostMsg::Response(request),
        )
        .await;

        if send_result.is_err() {
            error!("Failed to send response to MPTF request!");
        }
    }
}
