//! Definitions for standard MPTF messages the generic Thermal service can expect
//!
//! Transport services such as eSPI and SSH would need to ensure messages are sent to the Thermal service in this format.
//!
//! This interface is subject to change as the eSPI OOB service is developed
use crate::{self as ts, fan, sensor, utils};
use embedded_services::{comms, error};

/// MPTF Standard UUIDs which the thermal service understands
pub mod uuid_standard {
    pub const CRT_TEMP: uuid::Bytes = *uuid::uuid!("218246e7-baf6-45f1-aa13-07e4845256b8").as_bytes();
    pub const PROC_HOT_TEMP: uuid::Bytes = *uuid::uuid!("22dc52d2-fd0b-47ab-95b8-26552f9831a5").as_bytes();
    pub const PROFILE_TYPE: uuid::Bytes = *uuid::uuid!("23b4a025-cdfd-4af9-a411-37a24c574615").as_bytes();
    pub const FAN_ON_TEMP: uuid::Bytes = *uuid::uuid!("ba17b567-c368-48d5-bc6f-a312a41583c1").as_bytes();
    pub const FAN_RAMP_TEMP: uuid::Bytes = *uuid::uuid!("3a62688c-d95b-4d2d-bacc-90d7a5816bcd").as_bytes();
    pub const FAN_MAX_TEMP: uuid::Bytes = *uuid::uuid!("dcb758b1-f0fd-4ec7-b2c0-ef1e2a547b76").as_bytes();
    pub const FAN_MIN_RPM: uuid::Bytes = *uuid::uuid!("db261c77-934b-45e2-9742-256c62badb7a").as_bytes();
    pub const FAN_MAX_RPM: uuid::Bytes = *uuid::uuid!("5cf839df-8be7-42b9-9ac5-3403ca2c8a6a").as_bytes();
    pub const FAN_CURRENT_RPM: uuid::Bytes = *uuid::uuid!("adf95492-0776-4ffc-84f3-b6c8b5269683").as_bytes();
}

/// Standard 32-bit DWORD
pub type Dword = u32;

/// Thermalzone ID
pub type TzId = u8;

/// Time in miliseconds
pub type Miliseconds = Dword;

/// MPTF expects temperatures in tenth Kelvins
pub type DeciKelvin = Dword;

/// Helper MPTF Response type
pub type Response = Result<ResponseData, Error>;

/// Error codes expected by MPTF
#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Error {
    /// Invalid parameter was used
    InvalidParameter,
    /// Revision is not supported
    UnsupportedRevision,
    /// A hardware error occurred
    HardwareError,
}

impl From<Error> for u8 {
    fn from(value: Error) -> Self {
        match value {
            Error::InvalidParameter => 1,
            Error::UnsupportedRevision => 2,
            Error::HardwareError => 3,
        }
    }
}

/// Standard MPTF requests expected by the thermal subsystem
#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Request {
    /// EC_THM_GET_TMP
    GetTmp(TzId),
    /// EC_THM_GET_THRS
    GetThrs(TzId),
    /// EC_THM_SET_THRS
    SetThrs(TzId, Miliseconds, DeciKelvin, DeciKelvin),
    /// EC_THM_SET_SCP
    SetScp(TzId, Dword, Dword, Dword),
    /// EC_THM_GET_VAR
    GetVar(u8, uuid::Bytes),
    /// EC_THM_SET_VAR
    SetVar(u8, uuid::Bytes, Dword),
}

/// Data returned by thermal subsystem in response to MPTF requests  
#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum ResponseData {
    /// EC_THM_GET_TMP
    GetTmp(DeciKelvin),
    /// EC_THM_GET_THRS
    GetThrs(Miliseconds, DeciKelvin, DeciKelvin),
    /// EC_THM_SET_THRS
    SetThrs,
    /// EC_THM_SET_SCP
    SetScp,
    /// EC_THM_GET_VAR
    GetVar(u8, uuid::Bytes, Dword),
    /// EC_THM_SET_VAR
    SetVar,
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

async fn sensor_get_tmp(tzid: TzId) -> Response {
    match ts::execute_sensor_request(sensor::DeviceId(tzid), sensor::Request::GetTemp).await {
        Ok(ts::sensor::ResponseData::Temp(temp)) => Ok(ResponseData::GetTmp(utils::c_to_dk(temp))),
        _ => Err(Error::HardwareError),
    }
}

async fn sensor_get_warn_thrs(tzid: TzId) -> Response {
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
        (Ok(sensor::ResponseData::Threshold(low)), Ok(sensor::ResponseData::Threshold(high))) => {
            Ok(ResponseData::GetThrs(0, utils::c_to_dk(low), utils::c_to_dk(high)))
        }
        _ => Err(Error::HardwareError),
    }
}

async fn sensor_set_warn_thrs(tzid: TzId, _timeout: Dword, low: Dword, high: Dword) -> Response {
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
        Ok(ResponseData::SetThrs)
    } else {
        Err(Error::HardwareError)
    }
}

async fn sensor_get_thrs(instance: u8, threshold_type: sensor::ThresholdType, mptf_uid: uuid::Bytes) -> Response {
    match ts::execute_sensor_request(
        sensor::DeviceId(instance),
        sensor::Request::GetThreshold(threshold_type),
    )
    .await
    {
        Ok(sensor::ResponseData::Temp(temp)) => Ok(ResponseData::GetVar(instance, mptf_uid, utils::c_to_dk(temp))),
        _ => Err(Error::HardwareError),
    }
}

async fn fan_get_temp(instance: u8, fan_request: fan::Request, mptf_uid: uuid::Bytes) -> Response {
    match ts::execute_fan_request(fan::DeviceId(instance), fan_request).await {
        Ok(fan::ResponseData::Temp(temp)) => Ok(ResponseData::GetVar(instance, mptf_uid, utils::c_to_dk(temp))),
        _ => Err(Error::HardwareError),
    }
}

async fn fan_get_rpm(instance: u8, fan_request: fan::Request, mptf_uid: uuid::Bytes) -> Response {
    match ts::execute_fan_request(fan::DeviceId(instance), fan_request).await {
        Ok(fan::ResponseData::Rpm(rpm)) => Ok(ResponseData::GetVar(instance, mptf_uid, rpm as u32)),
        _ => Err(Error::HardwareError),
    }
}

async fn sensor_set_thrs(instance: u8, threshold_type: sensor::ThresholdType, threshold_dk: Dword) -> Response {
    match ts::execute_sensor_request(
        sensor::DeviceId(instance),
        sensor::Request::SetThreshold(threshold_type, utils::dk_to_c(threshold_dk)),
    )
    .await
    {
        Ok(sensor::ResponseData::Success) => Ok(ResponseData::SetVar),
        _ => Err(Error::HardwareError),
    }
}

async fn fan_set_var(instance: u8, fan_request: fan::Request) -> Response {
    match ts::execute_fan_request(fan::DeviceId(instance), fan_request).await {
        Ok(fan::ResponseData::Success) => Ok(ResponseData::SetVar),
        _ => Err(Error::HardwareError),
    }
}

#[embassy_executor::task]
pub async fn handle_requests() {
    loop {
        let response = match ts::wait_mptf_request().await {
            Request::GetTmp(tzid) => sensor_get_tmp(tzid).await,
            Request::GetThrs(tzid) => sensor_get_warn_thrs(tzid).await,
            Request::SetThrs(tzid, timeout, low, high) => sensor_set_warn_thrs(tzid, timeout, low, high).await,

            // TODO: How do we handle this genericly?
            Request::SetScp(_tzid, _policy_id, _acoustic_lim, _power_lim) => todo!(),

            Request::GetVar(instance, uuid_standard::CRT_TEMP) => {
                sensor_get_thrs(instance, sensor::ThresholdType::Critical, uuid_standard::CRT_TEMP).await
            }
            Request::GetVar(instance, uuid_standard::PROC_HOT_TEMP) => {
                sensor_get_thrs(instance, sensor::ThresholdType::Prochot, uuid_standard::PROC_HOT_TEMP).await
            }

            // TODO: Add a GetProfileId request type? But of sensor or fan?
            Request::GetVar(_instance, uuid_standard::PROFILE_TYPE) => todo!(),

            Request::GetVar(instance, uuid_standard::FAN_ON_TEMP) => {
                fan_get_temp(instance, fan::Request::GetOnTemp, uuid_standard::FAN_ON_TEMP).await
            }
            Request::GetVar(instance, uuid_standard::FAN_RAMP_TEMP) => {
                fan_get_temp(instance, fan::Request::GetRampTemp, uuid_standard::FAN_ON_TEMP).await
            }
            Request::GetVar(instance, uuid_standard::FAN_MAX_TEMP) => {
                fan_get_temp(instance, fan::Request::GetMaxTemp, uuid_standard::FAN_ON_TEMP).await
            }
            Request::GetVar(instance, uuid_standard::FAN_MIN_RPM) => {
                fan_get_rpm(instance, fan::Request::GetMinRpm, uuid_standard::FAN_MIN_RPM).await
            }
            Request::GetVar(instance, uuid_standard::FAN_MAX_RPM) => {
                fan_get_rpm(instance, fan::Request::GetMaxRpm, uuid_standard::FAN_MAX_RPM).await
            }
            Request::GetVar(instance, uuid_standard::FAN_CURRENT_RPM) => {
                fan_get_rpm(instance, fan::Request::GetRpm, uuid_standard::FAN_CURRENT_RPM).await
            }

            Request::SetVar(instance, uuid_standard::CRT_TEMP, temp_dk) => {
                sensor_set_thrs(instance, sensor::ThresholdType::Critical, temp_dk).await
            }
            Request::SetVar(instance, uuid_standard::PROC_HOT_TEMP, temp_dk) => {
                sensor_set_thrs(instance, sensor::ThresholdType::Critical, temp_dk).await
            }

            // TODO: Add a SetProfileId request type? But for sensor or fan?
            Request::SetVar(_instance, uuid_standard::PROFILE_TYPE, _profile_id) => todo!(),

            Request::SetVar(instance, uuid_standard::FAN_ON_TEMP, temp_dk) => {
                fan_set_var(instance, fan::Request::SetOnTemp(utils::dk_to_c(temp_dk))).await
            }
            Request::SetVar(instance, uuid_standard::FAN_RAMP_TEMP, temp_dk) => {
                fan_set_var(instance, fan::Request::SetRampTemp(utils::dk_to_c(temp_dk))).await
            }
            Request::SetVar(instance, uuid_standard::FAN_MAX_TEMP, temp_dk) => {
                fan_set_var(instance, fan::Request::SetMaxTemp(utils::dk_to_c(temp_dk))).await
            }

            // TODO: What does it mean to set the min/max RPM? Aren't these hardware defined?
            Request::SetVar(_instance, uuid_standard::FAN_MIN_RPM, _rpm) => todo!(),
            Request::SetVar(_instance, uuid_standard::FAN_MAX_RPM, _rpm) => todo!(),

            Request::SetVar(instance, uuid_standard::FAN_CURRENT_RPM, rpm) => {
                fan_set_var(instance, fan::Request::SetRpm(rpm as u16)).await
            }

            // TODO: Allow OEM to handle this?
            Request::GetVar(_instance, uuid) => {
                error!("Received GetVar for unrecognized UUID: {:?}", uuid);
                Err(Error::InvalidParameter)
            }
            Request::SetVar(_instance, uuid, _) => {
                error!("Received SetVar for unrecognized UUID: {:?}", uuid);
                Err(Error::InvalidParameter)
            }
        };

        if ts::send_service_msg(comms::EndpointID::External(comms::External::Host), &response)
            .await
            .is_err()
        {
            error!("Error responding to MPTF request!");
        }
    }
}
