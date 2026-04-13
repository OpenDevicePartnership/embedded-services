#![no_std]

mod serialization;

pub use serialization::{ThermalError, ThermalRequest, ThermalResponse, ThermalResult};
use thermal_service_interface::ThermalService;
use thermal_service_interface::fan::{self, FanService};
use thermal_service_interface::sensor::{self, SensorService};

/// DeciKelvin temperature representation.
///
/// This exists because the host to EC interface expects DeciKelvin,
/// though internally we still use Celsius for ease of use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct DeciKelvin(pub u32);

impl DeciKelvin {
    /// Convert from degrees Celsius to DeciKelvin.
    pub const fn from_celsius(c: f32) -> Self {
        Self(((c + 273.15) * 10.0) as u32)
    }

    /// Convert from DeciKelvin to degrees Celsius.
    pub const fn to_celsius(self) -> f32 {
        (self.0 as f32 / 10.0) - 273.15
    }
}

/// MPTF Standard UUIDs which the thermal service understands.
pub mod uuid_standard {
    /// The critical temperature threshold of a sensor.
    pub const CRT_TEMP: uuid::Bytes = uuid::uuid!("218246e7-baf6-45f1-aa13-07e4845256b8").to_bytes_le();
    /// The prochot temperature threshold of a sensor.
    pub const PROC_HOT_TEMP: uuid::Bytes = uuid::uuid!("22dc52d2-fd0b-47ab-95b8-26552f9831a5").to_bytes_le();
    /// The temperature threshold at which a fan should turn on and begin running at its minimum RPM.
    pub const FAN_MIN_TEMP: uuid::Bytes = uuid::uuid!("ba17b567-c368-48d5-bc6f-a312a41583c1").to_bytes_le();
    /// The temperature threshold at which a fan should start ramping up.
    pub const FAN_RAMP_TEMP: uuid::Bytes = uuid::uuid!("3a62688c-d95b-4d2d-bacc-90d7a5816bcd").to_bytes_le();
    /// The temperature threshold at which a fan should be at max speed.
    pub const FAN_MAX_TEMP: uuid::Bytes = uuid::uuid!("dcb758b1-f0fd-4ec7-b2c0-ef1e2a547b76").to_bytes_le();
    /// The minimum RPM a fan is capable of running at reliably.
    pub const FAN_MIN_RPM: uuid::Bytes = uuid::uuid!("db261c77-934b-45e2-9742-256c62badb7a").to_bytes_le();
    /// The maximum RPM a fan is capable of running at reliably.
    pub const FAN_MAX_RPM: uuid::Bytes = uuid::uuid!("5cf839df-8be7-42b9-9ac5-3403ca2c8a6a").to_bytes_le();
    /// The current RPM of a fan.
    pub const FAN_CURRENT_RPM: uuid::Bytes = uuid::uuid!("adf95492-0776-4ffc-84f3-b6c8b5269683").to_bytes_le();
}

/// Thermal service relay handler which wraps a thermal service instance.
pub struct ThermalServiceRelayHandler<T: ThermalService> {
    service: T,
}

impl<T: ThermalService> ThermalServiceRelayHandler<T> {
    /// Create a new thermal service relay handler.
    pub fn new(service: T) -> Self {
        Self { service }
    }

    async fn sensor_get_tmp(&self, instance_id: u8) -> ThermalResult {
        let sensor = self.service.sensor(instance_id).ok_or(ThermalError::InvalidParameter)?;
        let temp = sensor.temperature().await;
        Ok(ThermalResponse::ThermalGetTmpResponse {
            temperature: DeciKelvin::from_celsius(temp),
        })
    }

    async fn sensor_set_warn_thrs(
        &self,
        instance_id: u8,
        _timeout: u32,
        low: DeciKelvin,
        high: DeciKelvin,
    ) -> ThermalResult {
        let sensor = self.service.sensor(instance_id).ok_or(ThermalError::InvalidParameter)?;
        sensor.set_threshold(sensor::Threshold::WarnLow, low.to_celsius()).await;
        sensor
            .set_threshold(sensor::Threshold::WarnHigh, high.to_celsius())
            .await;
        Ok(ThermalResponse::ThermalSetThrsResponse)
    }

    async fn get_var_handler(&self, instance_id: u8, var_uuid: uuid::Bytes) -> ThermalResult {
        match var_uuid {
            uuid_standard::CRT_TEMP => self.sensor_get_thrs(instance_id, sensor::Threshold::Critical).await,
            uuid_standard::PROC_HOT_TEMP => self.sensor_get_thrs(instance_id, sensor::Threshold::Prochot).await,
            uuid_standard::FAN_MIN_TEMP => self.fan_get_state_temp(instance_id, fan::OnState::Min).await,
            uuid_standard::FAN_RAMP_TEMP => self.fan_get_state_temp(instance_id, fan::OnState::Ramping).await,
            uuid_standard::FAN_MAX_TEMP => self.fan_get_state_temp(instance_id, fan::OnState::Max).await,
            uuid_standard::FAN_MIN_RPM => self.fan_get_min_rpm(instance_id).await,
            uuid_standard::FAN_MAX_RPM => self.fan_get_max_rpm(instance_id).await,
            uuid_standard::FAN_CURRENT_RPM => self.fan_get_rpm(instance_id).await,
            _ => Err(ThermalError::InvalidParameter),
        }
    }

    async fn set_var_handler(&self, instance_id: u8, var_uuid: uuid::Bytes, set_var: u32) -> ThermalResult {
        match var_uuid {
            uuid_standard::CRT_TEMP => {
                self.sensor_set_thrs(instance_id, sensor::Threshold::Critical, set_var)
                    .await
            }
            uuid_standard::PROC_HOT_TEMP => {
                self.sensor_set_thrs(instance_id, sensor::Threshold::Prochot, set_var)
                    .await
            }
            uuid_standard::FAN_MIN_TEMP => {
                self.fan_set_state_temp(instance_id, fan::OnState::Min, DeciKelvin(set_var))
                    .await
            }
            uuid_standard::FAN_RAMP_TEMP => {
                self.fan_set_state_temp(instance_id, fan::OnState::Ramping, DeciKelvin(set_var))
                    .await
            }
            uuid_standard::FAN_MAX_TEMP => {
                self.fan_set_state_temp(instance_id, fan::OnState::Max, DeciKelvin(set_var))
                    .await
            }
            uuid_standard::FAN_CURRENT_RPM => {
                let rpm = u16::try_from(set_var).map_err(|_| ThermalError::InvalidParameter)?;
                self.fan_set_rpm(instance_id, rpm).await
            }
            _ => Err(ThermalError::InvalidParameter),
        }
    }

    async fn fan_get_state_temp(&self, instance_id: u8, state: fan::OnState) -> ThermalResult {
        let fan = self.service.fan(instance_id).ok_or(ThermalError::InvalidParameter)?;
        let temp = fan.state_temp(state).await;
        Ok(ThermalResponse::ThermalGetVarResponse {
            val: DeciKelvin::from_celsius(temp).0,
        })
    }

    async fn fan_get_rpm(&self, instance_id: u8) -> ThermalResult {
        let fan = self.service.fan(instance_id).ok_or(ThermalError::InvalidParameter)?;
        let rpm = fan.rpm().await;
        Ok(ThermalResponse::ThermalGetVarResponse { val: rpm.into() })
    }

    async fn fan_get_min_rpm(&self, instance_id: u8) -> ThermalResult {
        let fan = self.service.fan(instance_id).ok_or(ThermalError::InvalidParameter)?;
        let rpm = fan.min_rpm().await;
        Ok(ThermalResponse::ThermalGetVarResponse { val: rpm.into() })
    }

    async fn fan_get_max_rpm(&self, instance_id: u8) -> ThermalResult {
        let fan = self.service.fan(instance_id).ok_or(ThermalError::InvalidParameter)?;
        let rpm = fan.max_rpm().await;
        Ok(ThermalResponse::ThermalGetVarResponse { val: rpm.into() })
    }

    async fn sensor_set_thrs(&self, instance_id: u8, threshold: sensor::Threshold, threshold_dk: u32) -> ThermalResult {
        let sensor = self.service.sensor(instance_id).ok_or(ThermalError::InvalidParameter)?;
        sensor
            .set_threshold(threshold, DeciKelvin(threshold_dk).to_celsius())
            .await;
        Ok(ThermalResponse::ThermalSetVarResponse)
    }

    async fn sensor_get_thrs(&self, instance_id: u8, threshold: sensor::Threshold) -> ThermalResult {
        let sensor = self.service.sensor(instance_id).ok_or(ThermalError::InvalidParameter)?;
        let temp = sensor.threshold(threshold).await;
        Ok(ThermalResponse::ThermalGetVarResponse {
            val: DeciKelvin::from_celsius(temp).0,
        })
    }

    async fn sensor_get_warn_thrs(&self, instance_id: u8) -> ThermalResult {
        let sensor = self.service.sensor(instance_id).ok_or(ThermalError::InvalidParameter)?;
        let low = sensor.threshold(sensor::Threshold::WarnLow).await;
        let high = sensor.threshold(sensor::Threshold::WarnHigh).await;
        Ok(ThermalResponse::ThermalGetThrsResponse {
            timeout: 0,
            low: DeciKelvin::from_celsius(low),
            high: DeciKelvin::from_celsius(high),
        })
    }

    async fn fan_set_state_temp(&self, instance_id: u8, state: fan::OnState, temp: DeciKelvin) -> ThermalResult {
        let fan = self.service.fan(instance_id).ok_or(ThermalError::InvalidParameter)?;
        fan.set_state_temp(state, temp.to_celsius()).await;
        Ok(ThermalResponse::ThermalSetVarResponse)
    }

    async fn fan_set_rpm(&self, instance_id: u8, rpm: u16) -> ThermalResult {
        let fan = self.service.fan(instance_id).ok_or(ThermalError::InvalidParameter)?;
        fan.set_rpm(rpm).await.map_err(|_| ThermalError::HardwareError)?;
        Ok(ThermalResponse::ThermalSetVarResponse)
    }
}

impl<T: ThermalService> embedded_services::relay::mctp::RelayServiceHandlerTypes for ThermalServiceRelayHandler<T> {
    type RequestType = ThermalRequest;
    type ResultType = ThermalResult;
}

impl<T: ThermalService> embedded_services::relay::mctp::RelayServiceHandler for ThermalServiceRelayHandler<T> {
    async fn process_request(&self, request: Self::RequestType) -> Self::ResultType {
        match request {
            ThermalRequest::ThermalGetTmpRequest { instance_id } => self.sensor_get_tmp(instance_id).await,
            ThermalRequest::ThermalSetThrsRequest {
                instance_id,
                timeout,
                low,
                high,
            } => self.sensor_set_warn_thrs(instance_id, timeout, low, high).await,
            ThermalRequest::ThermalGetThrsRequest { instance_id } => self.sensor_get_warn_thrs(instance_id).await,
            // Revisit: Don't currently have a good strategy for handling this request
            ThermalRequest::ThermalSetScpRequest { .. } => Err(ThermalError::InvalidParameter),
            ThermalRequest::ThermalGetVarRequest {
                instance_id, var_uuid, ..
            } => self.get_var_handler(instance_id, var_uuid).await,
            ThermalRequest::ThermalSetVarRequest {
                instance_id,
                var_uuid,
                set_var,
                ..
            } => self.set_var_handler(instance_id, var_uuid, set_var).await,
        }
    }
}
