#![allow(dead_code)]
use core::ops::Deref;

use embedded_batteries_async::acpi::{PowerSourceState, PowerUnit};
use embedded_services::{
    debug,
    ec_type::message::{
        STD_BIX_BATTERY_SIZE, STD_BIX_MODEL_SIZE, STD_BIX_OEM_SIZE, STD_BIX_SERIAL_SIZE, STD_PIF_MODEL_SIZE,
        STD_PIF_OEM_SIZE, STD_PIF_SERIAL_SIZE, StdHostMsg, StdHostRequest,
    },
    ec_type::protocols::mctp,
    error, info,
    power::policy::PowerCapability,
    trace,
};

use crate::{
    context::PsuState,
    device::{DeviceId, DynamicBatteryMsgs, StaticBatteryMsgs},
};

#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub(crate) struct Payload<'a> {
    pub command: AcpiCmd,
    pub status: u8,
    pub data: &'a [u8],
}

#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub(crate) enum PayloadError {
    MalformedPayload,
    BufTooSmall(usize),
}

const ACPI_HEADER_SIZE: usize = 4;

#[derive(Copy, Clone)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub(crate) enum AcpiCmd {
    GetBix = 1,
    GetBst = 2,
    GetPsr = 3,
    GetPif = 4,
    GetBps = 5,
    SetBtp = 6,
    SetBpt = 7,
    GetBpc = 8,
    SetBmc = 9,
    GetBmd = 10,
    GetBct = 11,
    GetBtm = 12,
    SetBms = 13,
    SetBma = 14,
    GetSta = 15,
}

impl TryFrom<u8> for AcpiCmd {
    type Error = PayloadError;
    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(AcpiCmd::GetBix),
            2 => Ok(AcpiCmd::GetBst),
            3 => Ok(AcpiCmd::GetPsr),
            4 => Ok(AcpiCmd::GetPif),
            5 => Ok(AcpiCmd::GetBps),
            6 => Ok(AcpiCmd::SetBtp),
            7 => Ok(AcpiCmd::SetBpt),
            8 => Ok(AcpiCmd::GetBpc),
            9 => Ok(AcpiCmd::SetBmc),
            10 => Ok(AcpiCmd::GetBmd),
            11 => Ok(AcpiCmd::GetBct),
            12 => Ok(AcpiCmd::GetBtm),
            13 => Ok(AcpiCmd::SetBms),
            14 => Ok(AcpiCmd::SetBma),
            15 => Ok(AcpiCmd::GetSta),
            _ => Err(PayloadError::MalformedPayload),
        }
    }
}

impl From<AcpiCmd> for u8 {
    fn from(value: AcpiCmd) -> Self {
        value as u8
    }
}

pub(crate) fn compute_bst(cache: &DynamicBatteryMsgs) -> embedded_batteries_async::acpi::BstReturn {
    let charging = if cache.battery_status & (1 << 6) == 0 {
        embedded_batteries_async::acpi::BatteryState::CHARGING
    } else {
        embedded_batteries_async::acpi::BatteryState::DISCHARGING
    };

    // TODO: add critical energy state and charge limiting state
    embedded_batteries_async::acpi::BstReturn {
        battery_state: charging,
        battery_remaining_capacity: cache.remaining_capacity_mwh,
        battery_present_rate: cache.current_ma.unsigned_abs().into(),
        battery_present_voltage: cache.voltage_mv.into(),
    }
}

pub(crate) fn compute_bix<'a>(
    static_cache: &'a StaticBatteryMsgs,
    dynamic_cache: &'a DynamicBatteryMsgs,
) -> Result<mctp::BixFixedStrings<STD_BIX_MODEL_SIZE, STD_BIX_SERIAL_SIZE, STD_BIX_BATTERY_SIZE, STD_BIX_OEM_SIZE>, ()>
{
    let mut bix_return =
        mctp::BixFixedStrings::<STD_BIX_MODEL_SIZE, STD_BIX_SERIAL_SIZE, STD_BIX_BATTERY_SIZE, STD_BIX_OEM_SIZE> {
            revision: 1,
            power_unit: if static_cache.battery_mode.capacity_mode() {
                PowerUnit::MilliWatts
            } else {
                PowerUnit::MilliAmps
            },
            design_capacity: static_cache.design_capacity_mwh,
            last_full_charge_capacity: dynamic_cache.full_charge_capacity_mwh,
            battery_technology: embedded_batteries_async::acpi::BatteryTechnology::Secondary,
            design_voltage: static_cache.design_voltage_mv.into(),
            design_cap_of_warning: static_cache.design_cap_warning,
            design_cap_of_low: static_cache.design_cap_low,
            cycle_count: dynamic_cache.cycle_count.into(),
            measurement_accuracy: u32::from(100 - dynamic_cache.max_error_pct) * 1000u32,
            max_sampling_time: static_cache.max_sample_time,
            min_sampling_time: static_cache.min_sample_time,
            max_averaging_interval: static_cache.max_averaging_interval,
            min_averaging_interval: static_cache.min_averaging_interval,
            battery_capacity_granularity_1: static_cache.cap_granularity_1,
            battery_capacity_granularity_2: static_cache.cap_granularity_2,
            model_number: [0u8; STD_BIX_MODEL_SIZE],
            serial_number: [0u8; STD_BIX_SERIAL_SIZE],
            battery_type: [0u8; STD_BIX_BATTERY_SIZE],
            oem_info: [0u8; STD_BIX_OEM_SIZE],
            battery_swapping_capability: embedded_batteries_async::acpi::BatterySwapCapability::NonSwappable,
        };

    let model_number_len = core::cmp::min(STD_BIX_MODEL_SIZE - 1, static_cache.device_name.len() - 1);
    bix_return
        .model_number
        .get_mut(..model_number_len)
        .ok_or(())?
        .copy_from_slice(static_cache.device_name.get(..model_number_len).ok_or(())?);

    let serial_number_len = core::cmp::min(STD_BIX_SERIAL_SIZE - 1, static_cache.serial_num.len() - 1);
    bix_return
        .serial_number
        .get_mut(..serial_number_len)
        .ok_or(())?
        .copy_from_slice(static_cache.serial_num.get(..serial_number_len).ok_or(())?);

    let battery_type_len = core::cmp::min(STD_BIX_BATTERY_SIZE - 1, static_cache.device_chemistry.len() - 1);
    bix_return
        .battery_type
        .get_mut(..battery_type_len)
        .ok_or(())?
        .copy_from_slice(static_cache.device_chemistry.get(..battery_type_len).ok_or(())?);

    let oem_info_len = core::cmp::min(STD_BIX_OEM_SIZE - 1, static_cache.manufacturer_name.len() - 1);
    bix_return
        .oem_info
        .get_mut(..oem_info_len)
        .ok_or(())?
        .copy_from_slice(static_cache.manufacturer_name.get(..oem_info_len).ok_or(())?);

    Ok(bix_return)
}

pub(crate) fn compute_bps(dynamic_cache: &DynamicBatteryMsgs) -> embedded_batteries_async::acpi::Bps {
    // TODO: period values are correct for bq40z50, add to config to support other fuel gauges
    embedded_batteries_async::acpi::Bps {
        revision: 1,
        instantaneous_peak_power_level: dynamic_cache.max_power_mw,
        instantaneous_peak_power_period: 10,
        sustainable_peak_power_level: dynamic_cache.sus_power_mw,
        sustainable_peak_power_period: 10000,
    }
}

pub(crate) fn compute_bpc(static_cache: &StaticBatteryMsgs) -> embedded_batteries_async::acpi::Bpc {
    embedded_batteries_async::acpi::Bpc {
        revision: 1,
        power_threshold_support: static_cache.power_threshold_support,
        max_instantaneous_peak_power_threshold: static_cache.max_instant_pwr_threshold,
        max_sustainable_peak_power_threshold: static_cache.max_sus_pwr_threshold,
    }
}

pub(crate) fn compute_bmd(
    static_cache: &StaticBatteryMsgs,
    dynamic_cache: &DynamicBatteryMsgs,
) -> embedded_batteries_async::acpi::Bmd {
    embedded_batteries_async::acpi::Bmd {
        status_flags: dynamic_cache.bmd_status,
        capability_flags: static_cache.bmd_capability,
        recalibrate_count: static_cache.bmd_recalibrate_count,
        quick_recalibrate_time: static_cache.bmd_quick_recalibrate_time,
        slow_recalibrate_time: static_cache.bmd_slow_recalibrate_time,
    }
}

pub(crate) fn compute_bct(
    payload: &embedded_batteries_async::acpi::Bct,
    _dynamic_cache: &DynamicBatteryMsgs,
) -> embedded_batteries_async::acpi::BctReturnResult {
    // Just echo back charge level for now
    // TODO: Actually compute time from charge level
    embedded_batteries_async::acpi::BctReturnResult::from(payload.charge_level_percent)
}

pub(crate) fn compute_btm(
    payload: &embedded_batteries_async::acpi::Btm,
    _dynamic_cache: &DynamicBatteryMsgs,
) -> embedded_batteries_async::acpi::BtmReturnResult {
    // Just echo back charge level for now
    // TODO: Actually compute time from charge level
    embedded_batteries_async::acpi::BtmReturnResult::from(payload.discharge_rate)
}

pub(crate) fn compute_sta() -> embedded_batteries_async::acpi::StaReturn {
    // TODO: Grab real state values
    embedded_batteries_async::acpi::StaReturn::all()
}

pub(crate) fn compute_psr(psu_state: &PsuState) -> embedded_batteries_async::acpi::PsrReturn {
    // TODO: Refactor to check if battery if force discharged,
    // which should give an offline result even when the PSU is attached.
    embedded_batteries_async::acpi::PsrReturn {
        power_source: if psu_state.psu_connected {
            embedded_batteries_async::acpi::PowerSource::Online
        } else {
            embedded_batteries_async::acpi::PowerSource::Offline
        },
    }
}

pub(crate) fn compute_pif(
    psu_state: &PsuState,
) -> mctp::PifFixedStrings<STD_PIF_MODEL_SIZE, STD_PIF_SERIAL_SIZE, STD_PIF_OEM_SIZE> {
    // TODO: Grab real values from power policy
    let capability = psu_state.power_capability.unwrap_or(PowerCapability {
        voltage_mv: 0,
        current_ma: 0,
    });

    mctp::PifFixedStrings {
        power_source_state: PowerSourceState::empty(),
        max_output_power: capability.max_power_mw(),
        max_input_power: capability.max_power_mw(),
        model_number: [0u8; STD_PIF_MODEL_SIZE],
        serial_number: [0u8; STD_PIF_SERIAL_SIZE],
        oem_info: [0u8; STD_PIF_OEM_SIZE],
    }
}

impl crate::context::Context {
    // TODO Move these to a trait
    pub(super) async fn bix_handler(&self, request: &mut StdHostRequest) {
        trace!("Battery service: got BIX command!");
        // Enough space for all string fields to have 7 bytes + 1 null terminator byte
        match request.payload {
            mctp::Odp::BatteryGetBixRequest { battery_id } => {
                if let Some(fg) = self.get_fuel_gauge(DeviceId(battery_id)) {
                    let static_cache_guard = fg.get_static_battery_cache_guarded().await;
                    let dynamic_cache_guard = fg.get_dynamic_battery_cache_guarded().await;
                    request.payload = mctp::Odp::BatteryGetBixResponse {
                        bix: match compute_bix(static_cache_guard.deref(), dynamic_cache_guard.deref()) {
                            Ok(bix) => bix,
                            Err(()) => {
                                error!("Battery service: Failed to compute BIX");
                                // Drop locks before next await point to eliminate possibility of deadlock
                                drop(static_cache_guard);
                                drop(dynamic_cache_guard);

                                request.status = 1;
                                request.payload = mctp::Odp::ErrorResponse {};

                                super::comms_send(
                                    crate::EndpointID::External(embedded_services::comms::External::Host),
                                    request,
                                )
                                .await
                                .unwrap();
                                debug!("response sent to espi_service");
                                return;
                            }
                        },
                    };
                    // Drop locks before next await point to eliminate possibility of deadlock
                    drop(static_cache_guard);
                    drop(dynamic_cache_guard);

                    request.status = 0;
                    super::comms_send(
                        crate::EndpointID::External(embedded_services::comms::External::Host),
                        &StdHostMsg::Response(*request),
                    )
                    .await
                    .unwrap();

                    debug!("response sent to espi_service");
                } else {
                    error!("Battery service: FG not found when trying to process ACPI cmd!");
                }
            }
            _ => error!("Battery service: command and body mismatch!"),
        }
    }

    pub(super) async fn bst_handler(&self, request: &mut StdHostRequest) {
        trace!("Battery service: got BST command!");
        match request.payload {
            mctp::Odp::BatteryGetBstRequest { battery_id } => {
                if let Some(fg) = self.get_fuel_gauge(DeviceId(battery_id)) {
                    request.payload = mctp::Odp::BatteryGetBstResponse {
                        bst: compute_bst(&fg.get_dynamic_battery_cache().await),
                    };
                    request.status = 0;
                } else {
                    error!("Battery service: FG not found when trying to process ACPI cmd!");
                    request.status = 1;
                    request.payload = mctp::Odp::ErrorResponse {};
                }
            }
            _ => error!("Battery service: command and body mismatch!"),
        }
        super::comms_send(
            crate::EndpointID::External(embedded_services::comms::External::Host),
            &StdHostMsg::Response(*request),
        )
        .await
        .unwrap();

        trace!("response sent to espi_service");
    }

    pub(super) async fn psr_handler(&self, request: &mut StdHostRequest) {
        trace!("Battery service: got PSR command!");

        match request.payload {
            mctp::Odp::BatteryGetPsrRequest { battery_id } => {
                if let Some(_fg) = self.get_fuel_gauge(DeviceId(battery_id)) {
                    request.payload = mctp::Odp::BatteryGetPsrResponse {
                        psr: compute_psr(&self.get_power_info().await),
                    };
                    request.status = 0;
                } else {
                    error!("Battery service: FG not found when trying to process ACPI cmd!");
                    request.status = 1;
                    request.payload = mctp::Odp::ErrorResponse {};
                }
            }
            _ => error!("Battery service: command and body mismatch!"),
        }

        super::comms_send(
            crate::EndpointID::External(embedded_services::comms::External::Host),
            &StdHostMsg::Response(*request),
        )
        .await
        .unwrap();
        trace!("response sent to espi_service");
    }

    pub(super) async fn pif_handler(&self, request: &mut StdHostRequest) {
        trace!("Battery service: got PIF command!");

        match request.payload {
            mctp::Odp::BatteryGetPifRequest { battery_id } => {
                if let Some(_fg) = self.get_fuel_gauge(DeviceId(battery_id)) {
                    request.payload = mctp::Odp::BatteryGetPifResponse {
                        pif: compute_pif(&self.get_power_info().await),
                    };
                    request.status = 0;
                } else {
                    error!("Battery service: FG not found when trying to process ACPI cmd!");
                    request.status = 1;
                    request.payload = mctp::Odp::ErrorResponse {};
                }
            }
            _ => error!("Battery service: command and body mismatch!"),
        }

        super::comms_send(
            crate::EndpointID::External(embedded_services::comms::External::Host),
            &StdHostMsg::Response(*request),
        )
        .await
        .unwrap();
        trace!("response sent to espi_service");
    }

    pub(super) async fn bps_handler(&self, request: &mut StdHostRequest) {
        trace!("Battery service: got BPS command!");

        match request.payload {
            mctp::Odp::BatteryGetBpsRequest { battery_id } => {
                if let Some(fg) = self.get_fuel_gauge(DeviceId(battery_id)) {
                    request.payload = mctp::Odp::BatteryGetBpsResponse {
                        bps: compute_bps(&fg.get_dynamic_battery_cache().await),
                    };
                    request.status = 0;
                } else {
                    error!("Battery service: FG not found when trying to process ACPI cmd!");
                    request.status = 1;
                    request.payload = mctp::Odp::ErrorResponse {};
                }
            }
            _ => error!("Battery service: command and body mismatch!"),
        }

        super::comms_send(
            crate::EndpointID::External(embedded_services::comms::External::Host),
            &StdHostMsg::Response(*request),
        )
        .await
        .unwrap();
    }

    pub(super) async fn btp_handler(&self, request: &mut StdHostRequest) {
        trace!("Battery service: got BTP command!");

        match request.payload {
            mctp::Odp::BatterySetBtpRequest { battery_id, btp } => {
                if let Some(_fg) = self.get_fuel_gauge(DeviceId(battery_id)) {
                    // TODO: Save trip point
                    info!("Battery service: New BTP {}", btp.trip_point);
                    request.payload = mctp::Odp::BatterySetBtpResponse {};
                    request.status = 0;
                } else {
                    error!("Battery service: FG not found when trying to process ACPI cmd!");
                    request.status = 1;
                    request.payload = mctp::Odp::ErrorResponse {};
                }
            }
            _ => error!("Battery service: command and body mismatch!"),
        }

        super::comms_send(
            crate::EndpointID::External(embedded_services::comms::External::Host),
            &StdHostMsg::Response(*request),
        )
        .await
        .unwrap();
    }

    pub(super) async fn bpt_handler(&self, request: &mut StdHostRequest) {
        trace!("Battery service: got BPT command!");

        match request.payload {
            mctp::Odp::BatterySetBptRequest { battery_id, bpt } => {
                if let Some(_fg) = self.get_fuel_gauge(DeviceId(battery_id)) {
                    info!(
                        "Battery service: Threshold ID: {:?}, Threshold value: {:?}",
                        bpt.threshold_id as u32, bpt.threshold_value
                    );
                    request.payload = mctp::Odp::BatterySetBptResponse {};
                    request.status = 0;
                } else {
                    error!("Battery service: FG not found when trying to process ACPI cmd!");
                    request.status = 1;
                    request.payload = mctp::Odp::ErrorResponse {};
                }
            }
            _ => error!("Battery service: command and body mismatch!"),
        }

        super::comms_send(
            crate::EndpointID::External(embedded_services::comms::External::Host),
            &StdHostMsg::Response(*request),
        )
        .await
        .unwrap();
    }

    pub(super) async fn bpc_handler(&self, request: &mut StdHostRequest) {
        trace!("Battery service: got BPC command!");

        match request.payload {
            mctp::Odp::BatteryGetBpcRequest { battery_id } => {
                if let Some(fg) = self.get_fuel_gauge(DeviceId(battery_id)) {
                    // TODO: Save trip point
                    request.payload = mctp::Odp::BatteryGetBpcResponse {
                        bpc: compute_bpc(&fg.get_static_battery_cache().await),
                    };
                    request.status = 0;
                } else {
                    error!("Battery service: FG not found when trying to process ACPI cmd!");
                    request.status = 1;
                    request.payload = mctp::Odp::ErrorResponse {};
                }
            }
            _ => error!("Battery service: command and body mismatch!"),
        }

        super::comms_send(
            crate::EndpointID::External(embedded_services::comms::External::Host),
            &StdHostMsg::Response(*request),
        )
        .await
        .unwrap();
    }

    pub(super) async fn bmc_handler(&self, request: &mut StdHostRequest) {
        trace!("Battery service: got BMC command!");

        match request.payload {
            mctp::Odp::BatterySetBmcRequest { battery_id, bmc } => {
                if let Some(_fg) = self.get_fuel_gauge(DeviceId(battery_id)) {
                    info!("Battery service: Bmc {}", bmc.maintenance_control_flags.bits());
                    request.payload = mctp::Odp::BatterySetBmcResponse {};
                    request.status = 0;
                } else {
                    error!("Battery service: FG not found when trying to process ACPI cmd!");
                    request.status = 1;
                    request.payload = mctp::Odp::ErrorResponse {};
                }
            }
            _ => error!("Battery service: command and body mismatch!"),
        }

        super::comms_send(
            crate::EndpointID::External(embedded_services::comms::External::Host),
            &StdHostMsg::Response(*request),
        )
        .await
        .unwrap();
    }

    pub(super) async fn bmd_handler(&self, request: &mut StdHostRequest) {
        trace!("Battery service: got BMD command!");

        match request.payload {
            mctp::Odp::BatteryGetBmdRequest { battery_id } => {
                if let Some(fg) = self.get_fuel_gauge(DeviceId(battery_id)) {
                    let static_cache = fg.get_static_battery_cache().await;
                    let dynamic_cache = fg.get_dynamic_battery_cache().await;
                    request.payload = mctp::Odp::BatteryGetBmdResponse {
                        bmd: compute_bmd(&static_cache, &dynamic_cache),
                    };
                    request.status = 0;
                } else {
                    error!("Battery service: FG not found when trying to process ACPI cmd!");
                    request.status = 1;
                    request.payload = mctp::Odp::ErrorResponse {};
                }
            }
            _ => error!("Battery service: command and body mismatch!"),
        }

        super::comms_send(
            crate::EndpointID::External(embedded_services::comms::External::Host),
            &StdHostMsg::Response(*request),
        )
        .await
        .unwrap();
    }

    pub(super) async fn bct_handler(&self, request: &mut StdHostRequest) {
        trace!("Battery service: got BCT command!");

        match request.payload {
            mctp::Odp::BatteryGetBctRequest { battery_id, bct } => {
                if let Some(fg) = self.get_fuel_gauge(DeviceId(battery_id)) {
                    info!("Recvd BCT charge_level_percent: {}", bct.charge_level_percent);
                    request.payload = mctp::Odp::BatteryGetBctResponse {
                        bct_response: compute_bct(&bct, &fg.get_dynamic_battery_cache().await),
                    };
                    request.status = 0;
                } else {
                    error!("Battery service: FG not found when trying to process ACPI cmd!");
                    request.status = 1;
                    request.payload = mctp::Odp::ErrorResponse {};
                }
            }
            _ => error!("Battery service: command and body mismatch!"),
        }

        super::comms_send(
            crate::EndpointID::External(embedded_services::comms::External::Host),
            &StdHostMsg::Response(*request),
        )
        .await
        .unwrap();
    }

    pub(super) async fn btm_handler(&self, request: &mut StdHostRequest) {
        trace!("Battery service: got BTM command!");

        match request.payload {
            mctp::Odp::BatteryGetBtmRequest { battery_id, btm } => {
                if let Some(fg) = self.get_fuel_gauge(DeviceId(battery_id)) {
                    info!("Recvd BTM discharge_rate: {}", btm.discharge_rate);
                    request.payload = mctp::Odp::BatteryGetBtmResponse {
                        btm_response: compute_btm(&btm, &fg.get_dynamic_battery_cache().await),
                    };
                    request.status = 0;
                } else {
                    error!("Battery service: FG not found when trying to process ACPI cmd!");
                    request.status = 1;
                    request.payload = mctp::Odp::ErrorResponse {};
                }
            }
            _ => error!("Battery service: command and body mismatch!"),
        }

        super::comms_send(
            crate::EndpointID::External(embedded_services::comms::External::Host),
            &StdHostMsg::Response(*request),
        )
        .await
        .unwrap();
    }

    pub(super) async fn bms_handler(&self, request: &mut StdHostRequest) {
        trace!("Battery service: got BMS command!");

        match request.payload {
            mctp::Odp::BatterySetBmsRequest { battery_id, bms } => {
                if let Some(_fg) = self.get_fuel_gauge(DeviceId(battery_id)) {
                    info!("Recvd BMS sampling_time: {}", bms.sampling_time_ms);
                    request.payload = mctp::Odp::BatterySetBmsResponse { status: 0 };
                    request.status = 0;
                } else {
                    error!("Battery service: FG not found when trying to process ACPI cmd!");
                    request.status = 1;
                    request.payload = mctp::Odp::BatterySetBmsResponse { status: 1 };
                }
            }
            _ => error!("Battery service: command and body mismatch!"),
        }

        super::comms_send(
            crate::EndpointID::External(embedded_services::comms::External::Host),
            &StdHostMsg::Response(*request),
        )
        .await
        .unwrap();
    }

    pub(super) async fn bma_handler(&self, request: &mut StdHostRequest) {
        trace!("Battery service: got BMA command!");

        match request.payload {
            mctp::Odp::BatterySetBmaRequest { battery_id, bma } => {
                if let Some(_fg) = self.get_fuel_gauge(DeviceId(battery_id)) {
                    info!("Recvd BMA averaging_interval_ms: {}", bma.averaging_interval_ms);
                    request.payload = mctp::Odp::BatterySetBmaResponse { status: 0 };
                    request.status = 0;
                } else {
                    error!("Battery service: FG not found when trying to process ACPI cmd!");
                    request.status = 1;
                    request.payload = mctp::Odp::BatterySetBmaResponse { status: 1 };
                }
            }
            _ => error!("Battery service: command and body mismatch!"),
        }

        super::comms_send(
            crate::EndpointID::External(embedded_services::comms::External::Host),
            &StdHostMsg::Response(*request),
        )
        .await
        .unwrap();
    }

    pub(super) async fn sta_handler(&self, request: &mut StdHostRequest) {
        trace!("Battery service: got STA command!");

        match request.payload {
            mctp::Odp::BatteryGetStaRequest { battery_id } => {
                if let Some(_fg) = self.get_fuel_gauge(DeviceId(battery_id)) {
                    request.payload = mctp::Odp::BatteryGetStaResponse { sta: compute_sta() };
                    request.status = 0;
                } else {
                    error!("Battery service: FG not found when trying to process ACPI cmd!");
                    request.status = 1;
                    request.payload = mctp::Odp::ErrorResponse {};
                }
            }
            _ => error!("Battery service: command and body mismatch!"),
        }

        super::comms_send(
            crate::EndpointID::External(embedded_services::comms::External::Host),
            &StdHostMsg::Response(*request),
        )
        .await
        .unwrap();
    }
}
