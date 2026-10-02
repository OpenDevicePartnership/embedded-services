//! Relay types for the time-alarm service over HID.

use embedded_services::relay::hid::HidError;
use embedded_services::relay::hid::{
    GetHidReport, GetHidReportType, HidDevicePowerState, HidReport, HidReportDescriptor, ReportId, SetHidReport,
};
use embedded_services::{error, info};
use time_alarm_service_interface::{AcpiTimerId, AlarmTimerSeconds, TimeAlarmService};

mod serialization;

use serialization::{AlarmCurrentState, FeatureReportId, InputReportId, OutputReportId};

/// Anything that can make a Set report fail. The host is told about it through the Failed state of
/// the matching Get report rather than through the HID transport.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
enum SetReportError {
    /// The report payload didn't match the layout we published in our report descriptor.
    Malformed,

    /// The service rejected the requested value.
    Service,
}

impl From<serialization::ReportError> for SetReportError {
    fn from(_: serialization::ReportError) -> Self {
        Self::Malformed
    }
}

impl From<embedded_mcu_hal::time::DatetimeClockError> for SetReportError {
    fn from(_: embedded_mcu_hal::time::DatetimeClockError) -> Self {
        Self::Service
    }
}

/// Relay adapter that presents the time-alarm service to a HID transport service as a HidDevice.
pub struct TimeAlarmHidRelay<'s, Service: TimeAlarmService> {
    service: Service,

    descriptor: HidReportDescriptor<'static>,

    /// Sticky failure of the last SetAlarm report, reported back on the next GetAlarm read.
    alarm_failed: bool,

    /// Sticky failure of the last SetTime report, reported back on the next GetTime read.
    time_failed: bool,

    _phantom: core::marker::PhantomData<&'s ()>,
}

impl<'s, Service: TimeAlarmService> TimeAlarmHidRelay<'s, Service> {
    pub fn new(service: Service) -> Self {
        Self {
            service,

            // Panic safety: The HID report descriptor is a static constant and should always be valid.
            #[allow(clippy::expect_used)]
            descriptor: HidReportDescriptor::new(serialization::TIME_ALARM_HID_DESCRIPTOR)
                .expect("time alarm HID report descriptor is a static constant and should be valid"),

            alarm_failed: false,
            time_failed: false,
            _phantom: core::marker::PhantomData,
        }
    }

    fn apply_set_alarm(&self, timer_id: AcpiTimerId, data: &[u8]) -> Result<(), SetReportError> {
        let timer = serialization::deserialize_set_alarm_report(data)?;
        info!("Parsed alarm timer: {:?}", timer);

        self.service.set_timer_value(timer_id, timer)?;

        Ok(())
    }

    fn apply_set_policy(&self, data: &[u8]) -> Result<(), SetReportError> {
        let wake_policy = serialization::deserialize_set_debounce_report(data)?;
        info!("Parsed alarm wake policy: {:?}", wake_policy);

        self.service
            .set_expired_timer_policy(AcpiTimerId::AcPower, wake_policy)?;
        self.service
            .set_expired_timer_policy(AcpiTimerId::DcPower, wake_policy)?;

        Ok(())
    }

    fn apply_set_time(&self, data: &[u8]) -> Result<(), SetReportError> {
        let time = serialization::deserialize_set_time_report(data)?;
        info!("Setting time to {:?}", time);
        self.service.set_real_time(time)?;

        Ok(())
    }
}

impl<'s, Service: TimeAlarmService> embedded_services::relay::hid::HidDevice for TimeAlarmHidRelay<'s, Service> {
    type InputReportMaxSize = typenum::U10;
    type OutputReportMaxSize = typenum::U8;
    type FeatureReportMaxSize = typenum::U1;

    const MAX_REPORT_COUNT: u8 = 4;
    const MAX_DESCRIPTOR_LEN: usize = serialization::TIME_ALARM_HID_DESCRIPTOR.len();

    fn report_descriptor(&self) -> &HidReportDescriptor<'_> {
        &self.descriptor
    }

    async fn process_get_report<R>(
        &mut self,
        report_type: GetHidReportType,
        report_id: ReportId,
        process_report: impl AsyncFnOnce(GetHidReport<'_>) -> R,
    ) -> Result<R, HidError> {
        info!("Received command to get report with ID {:?}", report_id);

        match report_type {
            GetHidReportType::Input => match InputReportId::try_from(report_id).map_err(|_| HidError::TriggerReset)? {
                InputReportId::GetAlarm => {
                    info!("Received command to get alarm report");
                    let ac_timer = self.service.get_timer_value(AcpiTimerId::AcPower);
                    let dc_timer = self.service.get_timer_value(AcpiTimerId::DcPower);
                    let read_failed = ac_timer.is_err() || dc_timer.is_err();
                    if read_failed {
                        error!("Failed to read alarm timer values");
                    }
                    let ac_timer = ac_timer.unwrap_or(AlarmTimerSeconds::DISABLED);
                    let dc_timer = dc_timer.unwrap_or(AlarmTimerSeconds::DISABLED);

                    // HID-TAD doesn't support separate AC/DC timer debounces so they'll report the same thing
                    let wake_policy = self.service.get_expired_timer_policy(AcpiTimerId::AcPower);

                    let ac_state = self.service.get_wake_status(AcpiTimerId::AcPower);
                    let dc_state = self.service.get_wake_status(AcpiTimerId::DcPower);

                    let current_state = if self.alarm_failed || read_failed {
                        AlarmCurrentState::Failed
                    } else if ac_state.timer_triggered_wake() || dc_state.timer_triggered_wake() {
                        AlarmCurrentState::Signaled
                    } else if ac_state.timer_expired() || dc_state.timer_expired() {
                        AlarmCurrentState::Expired
                    } else if ac_timer != AlarmTimerSeconds::DISABLED || dc_timer != AlarmTimerSeconds::DISABLED {
                        AlarmCurrentState::Running
                    } else {
                        AlarmCurrentState::Cleared
                    };

                    let vendor_current_state = 0;
                    let report = serialization::serialize_get_alarm_report(
                        ac_timer,
                        dc_timer,
                        wake_policy,
                        current_state,
                        vendor_current_state,
                    );
                    let report = HidReport::new(report_id, &report);
                    Ok(process_report(GetHidReport::Input(report)).await)
                }
                InputReportId::GetTime => {
                    info!("Received command to get time report");
                    let report = match self.service.get_real_time() {
                        Ok(time) if !self.time_failed => {
                            serialization::serialize_get_time_report(time, serialization::TimeCurrentState::Running, 0)
                        }
                        Ok(time) => {
                            serialization::serialize_get_time_report(time, serialization::TimeCurrentState::Failed, 0)
                        }
                        Err(_) => {
                            error!("Failed to read the current time");
                            serialization::serialize_failed_get_time_report(0)
                        }
                    };
                    let report = HidReport::new(report_id, &report);
                    Ok(process_report(GetHidReport::Input(report)).await)
                }
            },
            GetHidReportType::Feature => {
                match FeatureReportId::try_from(report_id).map_err(|_| HidError::TriggerReset)? {
                    FeatureReportId::Capabilities => {
                        let capabilities = self.service.get_capabilities();
                        let deepest_power_state =
                            if capabilities.ac_s5_wake_supported() || capabilities.dc_s5_wake_supported() {
                                serialization::PowerState::S5
                            } else if capabilities.ac_s4_wake_supported() || capabilities.dc_s4_wake_supported() {
                                serialization::PowerState::S4
                            } else {
                                serialization::PowerState::S3
                            };
                        let capabilities = serialization::serialize_capabilities_feature_report(deepest_power_state);
                        Ok(process_report(GetHidReport::Feature(HidReport::new(report_id, &capabilities))).await)
                    }
                }
            }
        }
    }

    async fn set_report(&mut self, report: &SetHidReport<'_>) -> Result<(), HidError> {
        match report {
            SetHidReport::Output(r) => {
                info!("Received command to set output report with ID {:?}", r.id());
                match OutputReportId::try_from(r.id()).map_err(|_| HidError::TriggerReset)? {
                    id @ (OutputReportId::SetAcAlarm | OutputReportId::SetDcAlarm) => {
                        let timer_id = if id == OutputReportId::SetAcAlarm {
                            AcpiTimerId::AcPower
                        } else {
                            AcpiTimerId::DcPower
                        };
                        info!("Received command to set {:?} alarm report", timer_id);
                        info!("Report data: {:?}", r.data());

                        self.alarm_failed = self
                            .apply_set_alarm(timer_id, r.data())
                            .inspect_err(|e| error!("Failed to apply SetAlarmReport: {:?}", e))
                            .is_err();
                    }
                    OutputReportId::SetDebounce => {
                        info!("Received command to set debounce report");
                        info!("Report data: {:?}", r.data());

                        self.alarm_failed = self
                            .apply_set_policy(r.data())
                            .inspect_err(|e| error!("Failed to apply SetDebounceReport: {:?}", e))
                            .is_err();
                    }
                    OutputReportId::SetTime => {
                        info!("Received command to set time report");
                        info!("Report data: {:?}", r.data());
                        self.time_failed = self
                            .apply_set_time(r.data())
                            .inspect_err(|e| error!("Failed to apply SetTimeReport: {:?}", e))
                            .is_err();
                    }
                }
            }
            SetHidReport::Feature(r) => info!(
                "NOT IMPLEMENTED: Received command to set feature report with ID {:?}",
                r.id()
            ),
        }
        Ok(())
    }

    async fn wait_for_input_report(&mut self) {
        // We don't emit any unsolicited input reports, so this will never complete. Wake is done out-of-band.
        core::future::pending::<()>().await;
    }

    async fn process_next_input_report<R>(
        &mut self,
        _process_report: impl AsyncFnOnce(HidReport<'_>) -> R,
    ) -> Result<R, HidError> {
        // We don't emit any unsolicited input reports, so this will never complete. Wake is done out-of-band.
        core::future::pending::<()>().await;
        Err(HidError::TriggerReset) // unreachable
    }

    fn has_pending_input_report(&mut self) -> bool {
        // We don't emit any unsolicited input reports, so this will never complete. Wake is done out-of-band.
        false
    }

    async fn set_power_state(&mut self, state: HidDevicePowerState) -> Result<(), HidError> {
        info!("Received command to set power state to {:?}", state);
        // We don't turn off when commanded because it's our job to wake the host out-of-band, even when we're "sleeping".
        Ok(())
    }

    async fn reset(&mut self) {
        info!("Received reset command");

        self.alarm_failed = false;
        self.time_failed = false;
    }
}
