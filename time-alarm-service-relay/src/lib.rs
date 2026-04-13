#![no_std]

use time_alarm_service_interface::TimeAlarmService;

mod serialization;
pub use serialization::{AcpiTimeAlarmRequest, AcpiTimeAlarmResponse, AcpiTimeAlarmResult};

/// A relay handler that converts MCTP messages into function calls against the time-alarm service.
pub struct TimeAlarmServiceRelayHandler<T: TimeAlarmService> {
    service: T,
}

impl<T: TimeAlarmService> TimeAlarmServiceRelayHandler<T> {
    /// Construct a new relay handler that transmits requests to the given time-alarm service.
    pub fn new(service: T) -> Self {
        Self { service }
    }
}

impl<T: TimeAlarmService> embedded_services::relay::mctp::RelayServiceHandlerTypes for TimeAlarmServiceRelayHandler<T> {
    type RequestType = AcpiTimeAlarmRequest;
    type ResultType = AcpiTimeAlarmResult;
}

impl<T: TimeAlarmService> embedded_services::relay::mctp::RelayServiceHandler for TimeAlarmServiceRelayHandler<T> {
    async fn process_request(&self, request: Self::RequestType) -> Self::ResultType {
        match request {
            AcpiTimeAlarmRequest::GetCapabilities => {
                Ok(AcpiTimeAlarmResponse::Capabilities(self.service.get_capabilities()))
            }
            AcpiTimeAlarmRequest::GetRealTime => Ok(AcpiTimeAlarmResponse::RealTime(self.service.get_real_time()?)),
            AcpiTimeAlarmRequest::SetRealTime(timestamp) => {
                self.service.set_real_time(timestamp)?;
                Ok(AcpiTimeAlarmResponse::OkNoData)
            }
            AcpiTimeAlarmRequest::GetWakeStatus(timer_id) => Ok(AcpiTimeAlarmResponse::TimerStatus(
                self.service.get_wake_status(timer_id),
            )),
            AcpiTimeAlarmRequest::ClearWakeStatus(timer_id) => {
                self.service.clear_wake_status(timer_id);
                Ok(AcpiTimeAlarmResponse::OkNoData)
            }
            AcpiTimeAlarmRequest::SetExpiredTimerPolicy(timer_id, timer_policy) => {
                self.service.set_expired_timer_policy(timer_id, timer_policy)?;
                Ok(AcpiTimeAlarmResponse::OkNoData)
            }
            AcpiTimeAlarmRequest::GetExpiredTimerPolicy(timer_id) => Ok(AcpiTimeAlarmResponse::WakePolicy(
                self.service.get_expired_timer_policy(timer_id),
            )),
            AcpiTimeAlarmRequest::SetTimerValue(timer_id, timer_value) => {
                self.service.set_timer_value(timer_id, timer_value)?;
                Ok(AcpiTimeAlarmResponse::OkNoData)
            }
            AcpiTimeAlarmRequest::GetTimerValue(timer_id) => Ok(AcpiTimeAlarmResponse::TimerSeconds(
                self.service.get_timer_value(timer_id)?,
            )),
        }
    }
}
