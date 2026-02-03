//! Definitions for standard MPTF messages the generic Thermal service can expect
//!
//! Transport services such as eSPI and SSH would need to ensure messages are sent to the Thermal service in this format.
//!
//! This interface is subject to change as the eSPI OOB service is developed
use crate::{self as ts, fan, sensor, utils};
use thermal_service_messages::{DeciKelvin, Milliseconds};

use embedded_services::error;

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

async fn sensor_get_tmp(
    instance_id: u8,
    thermal_service: &'static crate::Service,
) -> thermal_service_messages::ThermalResult {
    if let Ok(ts::sensor::ResponseData::Temp(temp)) = thermal_service
        .execute_sensor_request(sensor::DeviceId(instance_id), sensor::Request::GetTemp)
        .await
    {
        Ok(thermal_service_messages::ThermalResponse::ThermalGetTmpResponse {
            temperature: utils::c_to_dk(temp),
        })
    } else {
        Err(thermal_service_messages::ThermalError::InvalidParameter)
    }
}

async fn get_var_handler(
    instance_id: u8,
    var_uuid: uuid::Bytes,
    thermal_service: &'static crate::Service,
) -> thermal_service_messages::ThermalResult {
    match var_uuid {
        uuid_standard::CRT_TEMP => sensor_get_thrs(instance_id, sensor::ThresholdType::Critical, thermal_service).await,
        uuid_standard::PROC_HOT_TEMP => {
            sensor_get_thrs(instance_id, sensor::ThresholdType::Prochot, thermal_service).await
        }
        // TODO: Add a SetProfileId request type? But for sensor or fan?
        uuid_standard::PROFILE_TYPE => {
            todo!()
        }
        uuid_standard::FAN_ON_TEMP => fan_get_temp(instance_id, fan::Request::GetOnTemp, thermal_service).await,

        uuid_standard::FAN_RAMP_TEMP => fan_get_temp(instance_id, fan::Request::GetRampTemp, thermal_service).await,
        uuid_standard::FAN_MAX_TEMP => fan_get_temp(instance_id, fan::Request::GetMaxTemp, thermal_service).await,
        uuid_standard::FAN_MIN_RPM => fan_get_rpm(instance_id, fan::Request::GetMinRpm, thermal_service).await,
        uuid_standard::FAN_MAX_RPM => fan_get_rpm(instance_id, fan::Request::GetMaxRpm, thermal_service).await,
        uuid_standard::FAN_CURRENT_RPM => fan_get_rpm(instance_id, fan::Request::GetRpm, thermal_service).await,
        // TODO: Allow OEM to handle these?
        uuid => {
            error!("Received GetVar for unrecognized UUID: {:?}", uuid);
            Err(thermal_service_messages::ThermalError::InvalidParameter)
        }
    }
}

async fn set_var_handler(
    instance_id: u8,
    var_uuid: uuid::Bytes,
    set_var: u32,
    thermal_service: &'static crate::Service,
) -> thermal_service_messages::ThermalResult {
    match var_uuid {
        uuid_standard::CRT_TEMP => {
            sensor_set_thrs(instance_id, sensor::ThresholdType::Critical, set_var, thermal_service).await
        }
        uuid_standard::PROC_HOT_TEMP => {
            sensor_set_thrs(instance_id, sensor::ThresholdType::Prochot, set_var, thermal_service).await
        }
        // TODO: Add a SetProfileId request type? But for sensor or fan?
        uuid_standard::PROFILE_TYPE => {
            todo!()
        }
        uuid_standard::FAN_ON_TEMP => {
            fan_set_var(
                instance_id,
                fan::Request::SetOnTemp(utils::dk_to_c(set_var)),
                thermal_service,
            )
            .await
        }
        uuid_standard::FAN_RAMP_TEMP => {
            fan_set_var(
                instance_id,
                fan::Request::SetRampTemp(utils::dk_to_c(set_var)),
                thermal_service,
            )
            .await
        }
        uuid_standard::FAN_MAX_TEMP => {
            fan_set_var(
                instance_id,
                fan::Request::SetMaxTemp(utils::dk_to_c(set_var)),
                thermal_service,
            )
            .await
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
            fan_set_var(instance_id, fan::Request::SetRpm(set_var as u16), thermal_service).await
        }
        // TODO: Allow OEM to handle these?
        uuid => {
            error!("Received SetVar for unrecognized UUID: {:?}", uuid);
            Err(thermal_service_messages::ThermalError::InvalidParameter)
        }
    }
}

async fn sensor_get_warn_thrs(
    instance_id: u8,
    thermal_service: &'static crate::Service,
) -> thermal_service_messages::ThermalResult {
    let low = thermal_service
        .execute_sensor_request(
            sensor::DeviceId(instance_id),
            sensor::Request::GetThreshold(sensor::ThresholdType::WarnLow),
        )
        .await;
    let high = thermal_service
        .execute_sensor_request(
            sensor::DeviceId(instance_id),
            sensor::Request::GetThreshold(sensor::ThresholdType::WarnHigh),
        )
        .await;

    match (low, high) {
        (Ok(sensor::ResponseData::Threshold(low)), Ok(sensor::ResponseData::Threshold(high))) => {
            Ok(thermal_service_messages::ThermalResponse::ThermalGetThrsResponse {
                timeout: 0,
                low: utils::c_to_dk(low),
                high: utils::c_to_dk(high),
            })
        }
        _ => Err(thermal_service_messages::ThermalError::InvalidParameter),
    }
}

async fn sensor_set_warn_thrs(
    instance_id: u8,
    _timeout: Milliseconds,
    low: DeciKelvin,
    high: DeciKelvin,
    thermal_service: &'static crate::Service,
) -> thermal_service_messages::ThermalResult {
    let low_res = thermal_service
        .execute_sensor_request(
            sensor::DeviceId(instance_id),
            sensor::Request::SetThreshold(sensor::ThresholdType::WarnLow, utils::dk_to_c(low)),
        )
        .await;
    let high_res = thermal_service
        .execute_sensor_request(
            sensor::DeviceId(instance_id),
            sensor::Request::SetThreshold(sensor::ThresholdType::WarnHigh, utils::dk_to_c(high)),
        )
        .await;

    if low_res.is_ok() && high_res.is_ok() {
        Ok(thermal_service_messages::ThermalResponse::ThermalSetThrsResponse)
    } else {
        Err(thermal_service_messages::ThermalError::InvalidParameter)
    }
}

async fn sensor_get_thrs(
    instance: u8,
    threshold_type: sensor::ThresholdType,
    thermal_service: &'static crate::Service,
) -> thermal_service_messages::ThermalResult {
    match thermal_service
        .execute_sensor_request(
            sensor::DeviceId(instance),
            sensor::Request::GetThreshold(threshold_type),
        )
        .await
    {
        Ok(sensor::ResponseData::Temp(temp)) => Ok(thermal_service_messages::ThermalResponse::ThermalGetVarResponse {
            val: utils::c_to_dk(temp),
        }),
        _ => Err(thermal_service_messages::ThermalError::HardwareError),
    }
}

async fn fan_get_temp(
    instance: u8,
    fan_request: fan::Request,
    thermal_service: &'static crate::Service,
) -> thermal_service_messages::ThermalResult {
    match thermal_service
        .execute_fan_request(fan::DeviceId(instance), fan_request)
        .await
    {
        Ok(fan::ResponseData::Temp(temp)) => Ok(thermal_service_messages::ThermalResponse::ThermalGetVarResponse {
            val: utils::c_to_dk(temp),
        }),
        _ => Err(thermal_service_messages::ThermalError::HardwareError),
    }
}

async fn fan_get_rpm(
    instance: u8,
    fan_request: fan::Request,
    thermal_service: &'static crate::Service,
) -> thermal_service_messages::ThermalResult {
    match thermal_service
        .execute_fan_request(fan::DeviceId(instance), fan_request)
        .await
    {
        Ok(fan::ResponseData::Rpm(rpm)) => {
            Ok(thermal_service_messages::ThermalResponse::ThermalGetVarResponse { val: rpm.into() })
        }
        _ => Err(thermal_service_messages::ThermalError::HardwareError),
    }
}

async fn sensor_set_thrs(
    instance: u8,
    threshold_type: sensor::ThresholdType,
    threshold_dk: u32,
    thermal_service: &'static crate::Service,
) -> thermal_service_messages::ThermalResult {
    match thermal_service
        .execute_sensor_request(
            sensor::DeviceId(instance),
            sensor::Request::SetThreshold(threshold_type, utils::dk_to_c(threshold_dk)),
        )
        .await
    {
        Ok(sensor::ResponseData::Success) => Ok(thermal_service_messages::ThermalResponse::ThermalSetVarResponse),
        _ => Err(thermal_service_messages::ThermalError::HardwareError),
    }
}

async fn fan_set_var(
    instance: u8,
    fan_request: fan::Request,
    thermal_service: &'static crate::Service,
) -> thermal_service_messages::ThermalResult {
    match thermal_service
        .execute_fan_request(fan::DeviceId(instance), fan_request)
        .await
    {
        Ok(fan::ResponseData::Success) => Ok(thermal_service_messages::ThermalResponse::ThermalSetVarResponse),
        _ => Err(thermal_service_messages::ThermalError::HardwareError),
    }
}

pub(crate) async fn process_request(
    request: &thermal_service_messages::ThermalRequest,
    thermal_service: &'static crate::Service,
) -> thermal_service_messages::ThermalResult {
    match request {
        thermal_service_messages::ThermalRequest::ThermalGetTmpRequest { instance_id } => {
            sensor_get_tmp(*instance_id, thermal_service).await
        }
        thermal_service_messages::ThermalRequest::ThermalSetThrsRequest {
            instance_id,
            timeout,
            low,
            high,
        } => sensor_set_warn_thrs(*instance_id, *timeout, *low, *high, thermal_service).await,
        thermal_service_messages::ThermalRequest::ThermalGetThrsRequest { instance_id } => {
            sensor_get_warn_thrs(*instance_id, thermal_service).await
        }
        // TODO: How do we handle this generically?
        thermal_service_messages::ThermalRequest::ThermalSetScpRequest { .. } => todo!(),
        thermal_service_messages::ThermalRequest::ThermalGetVarRequest {
            instance_id,
            len: _len,
            var_uuid,
        } => get_var_handler(*instance_id, *var_uuid, thermal_service).await,
        thermal_service_messages::ThermalRequest::ThermalSetVarRequest {
            instance_id,
            len: _len,
            var_uuid,
            set_var,
        } => set_var_handler(*instance_id, *var_uuid, *set_var, thermal_service).await,
    }
}
