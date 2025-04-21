//! Definitions for standard MPTF messages the generic Thermal service can expect
//!
//! Transport services such as eSPI and SSH would need to ensure messages are sent to the Thermal service in this format.
//!
//! This interface is subject to change as the eSPI OOB service is developed

use crate::{self as ts, fan, sensor, utils};
use embedded_services::{comms, error};

/// MPTF Standard UIDs which the default service understands
// TODO: Put these UIDs into the actual correct format according to spec
pub mod uid {
    use super::Uid;
    pub const CRT_TEMP: Uid = 0x218246e7baf645f1aa1307e4845256b8;
    pub const PROC_HOT_TEMP: Uid = 0x22dc52d2fd0b47ab95b826552f9831a5;
    pub const PROFILE_TYPE: Uid = 0x23b4a025cdfd4af9a41137a24c574615;
    pub const FAN_ON_TEMP: Uid = 0xba17b567c36848d5bc6fa312a41583c1;
    pub const FAN_RAMP_TEMP: Uid = 0x3a62688cd95b4d2dbacc90d7a5816bcd;
    pub const FAN_MAX_TEMP: Uid = 0xdcb758b1f0fd4ec7b2c0ef1e2a547b76;
    pub const FAN_MIN_RPM: Uid = 0xdb261c77934b45e29742256c62badb7a;
    pub const FAN_MAX_RPM: Uid = 0x5cf839df8be742b99ac53403ca2c8a6a;
    pub const FAN_CURRENT_RPM: Uid = 0xadf9549207764ffc84f3b6c8b5269683;
}

/// Standard 32-bit DWORD
pub type Dword = u32;

/// Thermalzone ID
pub type TzId = u8;

/// Time in miliseconds
pub type Miliseconds = Dword;

/// MPTF expects temperatures in tenth Kelvins
pub type DeciKelvin = Dword;

/// UID underlying representation
pub type Uid = u128;

/// Helper MPTF Response type
pub type Response = Result<ResponseData, Error>;

/// Error codes expected by MPTF
#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Error {
    InvalidParameter,
    UnsupportedRevision,
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
    // EC_THM_GET_TMP
    GetTmp(TzId),
    // EC_THM_GET_THRS
    GetThrs(TzId),
    // EC_THM_SET_THRS
    SetThrs(TzId, Miliseconds, DeciKelvin, DeciKelvin),
    // EC_THM_SET_SCP
    SetScp(TzId, Dword, Dword, Dword),
    // EC_THM_GET_VAR
    GetVar(u8, Uid),
    // EC_THM_SET_VAR
    SetVar(u8, Uid, Dword),
}

/// Data returned by thermal subsystem in response to MPTF requests  
#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum ResponseData {
    // Standard command
    GetTmp(DeciKelvin),
    GetThrs(Miliseconds, DeciKelvin, DeciKelvin),
    SetThrs,
    SetScp,
    GetVar(u8, Uid, Dword),
    SetVar,
}

/// Notifications to Host
#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Notify {
    Warn,
    Critical,
    ProcHot,
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

async fn sensor_get_thrs(instance: u8, threshold_type: sensor::ThresholdType, mptf_uid: Uid) -> Response {
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

async fn fan_get_temp(instance: u8, fan_request: fan::Request, mptf_uid: Uid) -> Response {
    match ts::execute_fan_request(fan::DeviceId(instance), fan_request).await {
        Ok(fan::ResponseData::Temp(temp)) => Ok(ResponseData::GetVar(instance, mptf_uid, utils::c_to_dk(temp))),
        _ => Err(Error::HardwareError),
    }
}

async fn fan_get_rpm(instance: u8, fan_request: fan::Request, mptf_uid: Uid) -> Response {
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
    let response = match ts::wait_mptf_request().await {
        Request::GetTmp(tzid) => sensor_get_tmp(tzid).await,
        Request::GetThrs(tzid) => sensor_get_warn_thrs(tzid).await,
        Request::SetThrs(tzid, timeout, low, high) => sensor_set_warn_thrs(tzid, timeout, low, high).await,

        // TODO: How do we handle this genericly?
        Request::SetScp(_tzid, _policy_id, _acoustic_lim, _power_lim) => todo!(),

        Request::GetVar(instance, uid::CRT_TEMP) => {
            sensor_get_thrs(instance, sensor::ThresholdType::Critical, uid::CRT_TEMP).await
        }
        Request::GetVar(instance, uid::PROC_HOT_TEMP) => {
            sensor_get_thrs(instance, sensor::ThresholdType::Prochot, uid::PROC_HOT_TEMP).await
        }

        // TODO: Add a GetProfileId request type?
        Request::GetVar(_instance, uid::PROFILE_TYPE) => todo!(),

        Request::GetVar(instance, uid::FAN_ON_TEMP) => {
            fan_get_temp(instance, fan::Request::GetOnTemp, uid::FAN_ON_TEMP).await
        }
        Request::GetVar(instance, uid::FAN_RAMP_TEMP) => {
            fan_get_temp(instance, fan::Request::GetRampTemp, uid::FAN_ON_TEMP).await
        }
        Request::GetVar(instance, uid::FAN_MAX_TEMP) => {
            fan_get_temp(instance, fan::Request::GetMaxTemp, uid::FAN_ON_TEMP).await
        }
        Request::GetVar(instance, uid::FAN_MIN_RPM) => {
            fan_get_rpm(instance, fan::Request::GetMinRpm, uid::FAN_MIN_RPM).await
        }
        Request::GetVar(instance, uid::FAN_MAX_RPM) => {
            fan_get_rpm(instance, fan::Request::GetMaxRpm, uid::FAN_MAX_RPM).await
        }
        Request::GetVar(instance, uid::FAN_CURRENT_RPM) => {
            fan_get_rpm(instance, fan::Request::GetRpm, uid::FAN_CURRENT_RPM).await
        }

        Request::SetVar(instance, uid::CRT_TEMP, temp_dk) => {
            sensor_set_thrs(instance, sensor::ThresholdType::Critical, temp_dk).await
        }
        Request::SetVar(instance, uid::PROC_HOT_TEMP, temp_dk) => {
            sensor_set_thrs(instance, sensor::ThresholdType::Critical, temp_dk).await
        }

        // TODO: Add a SetProfileId request type?
        Request::SetVar(_instance, uid::PROFILE_TYPE, _profile_id) => todo!(),

        Request::SetVar(instance, uid::FAN_ON_TEMP, temp_dk) => {
            fan_set_var(instance, fan::Request::SetOnTemp(utils::dk_to_c(temp_dk))).await
        }
        Request::SetVar(instance, uid::FAN_RAMP_TEMP, temp_dk) => {
            fan_set_var(instance, fan::Request::SetRampTemp(utils::dk_to_c(temp_dk))).await
        }
        Request::SetVar(instance, uid::FAN_MAX_TEMP, temp_dk) => {
            fan_set_var(instance, fan::Request::SetMaxTemp(utils::dk_to_c(temp_dk))).await
        }

        // TODO: What does it mean to set the min/max RPM? Aren't these hardware defined?
        Request::SetVar(_instance, uid::FAN_MIN_RPM, _rpm) => todo!(),
        Request::SetVar(_instance, uid::FAN_MAX_RPM, _rpm) => todo!(),

        Request::SetVar(instance, uid::FAN_CURRENT_RPM, rpm) => {
            fan_set_var(instance, fan::Request::SetRpm(rpm as u16)).await
        }

        // TODO: Allow OEM to handle this?
        Request::GetVar(_instance, uid) => {
            error!("Received GetVar for unrecognized UID: {}", uid);
            Err(Error::InvalidParameter)
        }
        Request::SetVar(_instance, uid, _) => {
            error!("Received SetVar for unrecognized UID: {}", uid);
            Err(Error::InvalidParameter)
        }
    };

    ts::send_service_msg(comms::EndpointID::External(comms::External::Host), &response).await
}
