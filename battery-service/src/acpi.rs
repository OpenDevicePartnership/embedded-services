#![allow(dead_code)]
use core::ops::Deref;

use embedded_batteries_async::acpi::{PowerSourceState, PowerUnit};
use embedded_services::{info, power::policy::PowerCapability, trace};

use battery_service_messages::{
    AcpiBatteryResponse, BixFixedStrings, DeviceId, PifFixedStrings, STD_BIX_BATTERY_SIZE, STD_BIX_MODEL_SIZE,
    STD_BIX_OEM_SIZE, STD_BIX_SERIAL_SIZE, STD_PIF_MODEL_SIZE, STD_PIF_OEM_SIZE, STD_PIF_SERIAL_SIZE,
};

use crate::{
    AcpiBatteryError,
    context::PsuState,
    device::{DynamicBatteryMsgs, StaticBatteryMsgs},
};

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
) -> Result<BixFixedStrings, ()> {
    let mut bix_return = BixFixedStrings {
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

pub(crate) fn compute_pif(psu_state: &PsuState) -> PifFixedStrings {
    // TODO: Grab real values from power policy
    let capability = psu_state.power_capability.unwrap_or(PowerCapability {
        voltage_mv: 0,
        current_ma: 0,
    });

    PifFixedStrings {
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
    pub(super) async fn bix_handler(&self, device_id: DeviceId) -> Result<AcpiBatteryResponse, AcpiBatteryError> {
        trace!("Battery service: got BIX command!");

        let fg = self
            .get_fuel_gauge(device_id)
            .ok_or(AcpiBatteryError::UnknownDeviceId)?;

        let static_cache_guard = fg.get_static_battery_cache_guarded().await;
        let dynamic_cache_guard = fg.get_dynamic_battery_cache_guarded().await;

        Ok(AcpiBatteryResponse::BatteryGetBixResponse {
            bix: compute_bix(static_cache_guard.deref(), dynamic_cache_guard.deref())
                .map_err(|_| AcpiBatteryError::UnspecifiedFailure)?,
        })
    }

    pub(super) async fn bst_handler(&self, device_id: DeviceId) -> Result<AcpiBatteryResponse, AcpiBatteryError> {
        trace!("Battery service: got BST command!");

        let fg = self
            .get_fuel_gauge(device_id)
            .ok_or(AcpiBatteryError::UnknownDeviceId)?;

        Ok(AcpiBatteryResponse::BatteryGetBstResponse {
            bst: compute_bst(&fg.get_dynamic_battery_cache().await),
        })
    }

    pub(super) async fn psr_handler(&self, device_id: DeviceId) -> Result<AcpiBatteryResponse, AcpiBatteryError> {
        trace!("Battery service: got PSR command!");

        let _fg = self
            .get_fuel_gauge(device_id)
            .ok_or(AcpiBatteryError::UnknownDeviceId)?;

        // TODO @matteo why is this not using fg?
        Ok(AcpiBatteryResponse::BatteryGetPsrResponse {
            psr: compute_psr(&self.get_power_info().await),
        })
    }

    pub(super) async fn pif_handler(&self, device_id: DeviceId) -> Result<AcpiBatteryResponse, AcpiBatteryError> {
        trace!("Battery service: got PIF command!");

        let _fg = self
            .get_fuel_gauge(device_id)
            .ok_or(AcpiBatteryError::UnknownDeviceId)?;

        // TODO @matteo why is this not using fg?
        Ok(AcpiBatteryResponse::BatteryGetPifResponse {
            pif: compute_pif(&self.get_power_info().await),
        })
    }

    pub(super) async fn bps_handler(&self, device_id: DeviceId) -> Result<AcpiBatteryResponse, AcpiBatteryError> {
        trace!("Battery service: got BPS command!");

        let fg = self
            .get_fuel_gauge(device_id)
            .ok_or(AcpiBatteryError::UnknownDeviceId)?;

        Ok(AcpiBatteryResponse::BatteryGetBpsResponse {
            bps: compute_bps(&fg.get_dynamic_battery_cache().await),
        })
    }

    pub(super) async fn btp_handler(
        &self,
        device_id: DeviceId,
        btp: embedded_batteries_async::acpi::Btp,
    ) -> Result<AcpiBatteryResponse, AcpiBatteryError> {
        trace!("Battery service: got BTP command!");

        let _fg = self
            .get_fuel_gauge(device_id)
            .ok_or(AcpiBatteryError::UnknownDeviceId)?;

        // TODO: Save trip point
        info!("Battery service: New BTP {}", btp.trip_point);

        Ok(AcpiBatteryResponse::BatterySetBtpResponse {})
    }

    pub(super) async fn bpt_handler(
        &self,
        device_id: DeviceId,
        bpt: embedded_batteries_async::acpi::Bpt,
    ) -> Result<AcpiBatteryResponse, AcpiBatteryError> {
        trace!("Battery service: got BPT command!");

        let _fg = self
            .get_fuel_gauge(device_id)
            .ok_or(AcpiBatteryError::UnknownDeviceId)?;

        // TODO @matteo as far as I can tell, this never did anything - is this correct?
        info!(
            "Battery service: Threshold ID: {:?}, Threshold value: {:?}",
            bpt.threshold_id as u32, bpt.threshold_value
        );

        Ok(AcpiBatteryResponse::BatterySetBptResponse {})
    }

    pub(super) async fn bpc_handler(&self, device_id: DeviceId) -> Result<AcpiBatteryResponse, AcpiBatteryError> {
        trace!("Battery service: got BPC command!");

        let fg = self
            .get_fuel_gauge(device_id)
            .ok_or(AcpiBatteryError::UnknownDeviceId)?;

        Ok(AcpiBatteryResponse::BatteryGetBpcResponse {
            bpc: compute_bpc(&fg.get_static_battery_cache().await),
        })
    }

    pub(super) async fn bmc_handler(
        &self,
        device_id: DeviceId,
        bmc: embedded_batteries_async::acpi::Bmc,
    ) -> Result<AcpiBatteryResponse, AcpiBatteryError> {
        trace!("Battery service: got BMC command!");

        let _fg = self
            .get_fuel_gauge(device_id)
            .ok_or(AcpiBatteryError::UnknownDeviceId)?;

        // TODO @matteo as far as I can tell, this never did anything - is this correct?
        info!("Battery service: Bmc {}", bmc.maintenance_control_flags.bits());

        Ok(AcpiBatteryResponse::BatterySetBmcResponse {})
    }

    pub(super) async fn bmd_handler(&self, device_id: DeviceId) -> Result<AcpiBatteryResponse, AcpiBatteryError> {
        trace!("Battery service: got BMD command!");

        let fg = self
            .get_fuel_gauge(device_id)
            .ok_or(AcpiBatteryError::UnknownDeviceId)?;

        let static_cache = fg.get_static_battery_cache().await;
        let dynamic_cache = fg.get_dynamic_battery_cache().await;

        Ok(AcpiBatteryResponse::BatteryGetBmdResponse {
            bmd: compute_bmd(&static_cache, &dynamic_cache),
        })
    }

    pub(super) async fn bct_handler(
        &self,
        device_id: DeviceId,
        bct: embedded_batteries_async::acpi::Bct,
    ) -> Result<AcpiBatteryResponse, AcpiBatteryError> {
        trace!("Battery service: got BCT command!");

        let fg = self
            .get_fuel_gauge(device_id)
            .ok_or(AcpiBatteryError::UnknownDeviceId)?;

        info!("Recvd BCT charge_level_percent: {}", bct.charge_level_percent);
        Ok(AcpiBatteryResponse::BatteryGetBctResponse {
            bct_response: compute_bct(&bct, &fg.get_dynamic_battery_cache().await),
        })
    }

    pub(super) async fn btm_handler(
        &self,
        device_id: DeviceId,
        btm: embedded_batteries_async::acpi::Btm,
    ) -> Result<AcpiBatteryResponse, AcpiBatteryError> {
        trace!("Battery service: got BTM command!");

        let fg = self
            .get_fuel_gauge(device_id)
            .ok_or(AcpiBatteryError::UnknownDeviceId)?;

        info!("Recvd BTM discharge_rate: {}", btm.discharge_rate);
        Ok(AcpiBatteryResponse::BatteryGetBtmResponse {
            btm_response: compute_btm(&btm, &fg.get_dynamic_battery_cache().await),
        })
    }

    pub(super) async fn bms_handler(
        &self,
        device_id: DeviceId,
        bms: embedded_batteries_async::acpi::Bms,
    ) -> Result<AcpiBatteryResponse, AcpiBatteryError> {
        trace!("Battery service: got BMS command!");

        let _fg = self
            .get_fuel_gauge(device_id)
            .ok_or(AcpiBatteryError::UnknownDeviceId)?;

        // TODO @matteo this one handles errors differently than all the others - why?
        //      Previously this one returned a result with .status = 1 on failure as well as setting the failure status bit on hostmsg... what should we actually be returning here?
        // TODO @matteo this one doesn't seem to actually do anything in the success case - why?
        // TODO @matteo why does this not use _fg?
        info!("Recvd BMS sampling_time: {}", bms.sampling_time_ms);
        Ok(AcpiBatteryResponse::BatterySetBmsResponse { status: 0 })
    }

    pub(super) async fn bma_handler(
        &self,
        device_id: DeviceId,
        bma: embedded_batteries_async::acpi::Bma,
    ) -> Result<AcpiBatteryResponse, AcpiBatteryError> {
        trace!("Battery service: got BMA command!");

        let _fg = self
            .get_fuel_gauge(device_id)
            .ok_or(AcpiBatteryError::UnknownDeviceId)?;

        // TODO @matteo this one handles errors differently than all the others - why?
        //      Previously this one returned a result with .status = 1 on failure as well as setting the failure status bit on hostmsg... what should we actually be returning here?
        // TODO @matteo this one doesn't seem to actually do anything in the success case - why?
        // TODO @matteo this one doesn't use _fg - why?
        info!("Recvd BMA averaging_interval_ms: {}", bma.averaging_interval_ms);
        Ok(AcpiBatteryResponse::BatterySetBmaResponse { status: 0 })
    }

    pub(super) async fn sta_handler(&self, device_id: DeviceId) -> Result<AcpiBatteryResponse, AcpiBatteryError> {
        trace!("Battery service: got STA command!");

        let _fg = self
            .get_fuel_gauge(device_id)
            .ok_or(AcpiBatteryError::UnknownDeviceId)?;

        // TODO @matteo this looks odd to me - we're not actually using fg? why do we take it as a parameter?
        Ok(AcpiBatteryResponse::BatteryGetStaResponse { sta: compute_sta() })
    }
}
