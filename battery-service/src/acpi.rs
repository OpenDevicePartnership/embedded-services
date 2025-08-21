use core::{borrow::BorrowMut, ops::Deref};

use embedded_batteries_async::acpi::{
    BCT_RETURN_SIZE_BYTES, BMD_RETURN_SIZE_BYTES, BPC_RETURN_SIZE_BYTES, BPS_RETURN_SIZE_BYTES, BST_RETURN_SIZE_BYTES,
    BTM_RETURN_SIZE_BYTES, Bct, BctReturnResult, BixReturn, Bma, BmcControlFlags, Btm, BtmReturnResult,
    PSR_RETURN_SIZE_BYTES, Pif, PowerSourceState, PowerUnit, PsrReturn, STA_RETURN_SIZE_BYTES,
};
use embedded_services::{
    debug,
    ec_type::message::{AcpiMsgComms, HostMsg},
    error, info,
    power::policy::PowerCapability,
    trace,
};

use crate::{
    context::PsuState,
    device::{Device, DynamicBatteryMsgs, StaticBatteryMsgs},
};

#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub(crate) struct Payload<'a> {
    pub version: u8,
    pub instance: u8,
    pub reserved: u8,
    pub command: AcpiCmd,
    pub data: &'a [u8],
}

#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub(crate) enum PayloadError {
    MalformedPayload,
    BufTooSmall,
}

impl<'a> Payload<'a> {
    pub(crate) fn from_raw(raw: &'a [u8], size: usize) -> Result<Self, PayloadError> {
        Ok(Payload {
            version: raw[0],
            instance: raw[1],
            reserved: raw[2],
            command: AcpiCmd::try_from(raw[3])?,
            data: &raw[4..size],
        })
    }

    pub(crate) fn to_raw(&self, buf: &mut [u8]) -> Result<usize, PayloadError> {
        if buf.len() < self.data.len() + 4 {
            return Err(PayloadError::BufTooSmall);
        }

        buf[0] = self.version;
        buf[1] = self.instance;
        buf[2] = self.reserved;
        buf[3] = self.command as u8;
        buf[4..self.data.len() + 4].copy_from_slice(self.data);

        Ok(self.data.len() + 4)
    }
}

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

pub(crate) fn compute_bst(bst_return: &mut embedded_batteries_async::acpi::BstReturn, cache: &DynamicBatteryMsgs) {
    let charging = if cache.battery_status & (1 << 6) == 0 {
        embedded_batteries_async::acpi::BatteryState::CHARGING
    } else {
        embedded_batteries_async::acpi::BatteryState::DISCHARGING
    };

    // TODO: add critical energy state and charge limiting state
    bst_return.battery_state = charging;
    bst_return.battery_remaining_capacity = cache.remaining_capacity_mwh;
    bst_return.battery_present_rate = cache.current_ma.unsigned_abs().into();
    bst_return.battery_present_voltage = cache.voltage_mv.into();
}

pub(crate) fn compute_bix<'a>(
    static_cache: &'a StaticBatteryMsgs,
    dynamic_cache: &'a DynamicBatteryMsgs,
) -> BixReturn<'a> {
    embedded_batteries_async::acpi::BixReturn {
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
        model_number: &static_cache.device_name[..7],
        serial_number: &static_cache.serial_num,
        battery_type: &static_cache.device_chemistry,
        oem_info: &static_cache.manufacturer_name[..7],
        battery_swapping_capability: embedded_batteries_async::acpi::BatterySwapCapability::NonSwappable,
    }
}

pub(crate) fn compute_bps(bps_return: &mut embedded_batteries_async::acpi::Bps, dynamic_cache: &DynamicBatteryMsgs) {
    // TODO: period values are correct for bq40z50, add to config to support other fuel gauges
    bps_return.revision = 1;
    bps_return.instantaneous_peak_power_level = dynamic_cache.max_power_mw;
    bps_return.instantaneous_peak_power_period = 10;
    bps_return.sustainable_peak_power_level = dynamic_cache.sus_power_mw;
    bps_return.sustainable_peak_power_period = 10000;
}

pub(crate) fn compute_bpc(bpc_return: &mut embedded_batteries_async::acpi::Bpc, static_cache: &StaticBatteryMsgs) {
    bpc_return.revision = 1;
    bpc_return.power_threshold_support = static_cache.power_threshold_support;
    bpc_return.max_instantaneous_peak_power_threshold = static_cache.max_instant_pwr_threshold;
    bpc_return.max_sustainable_peak_power_threshold = static_cache.max_sus_pwr_threshold;
}

pub(crate) fn compute_bmd(
    bmd_return: &mut embedded_batteries_async::acpi::Bmd,
    static_cache: &StaticBatteryMsgs,
    dynamic_cache: &DynamicBatteryMsgs,
) {
    bmd_return.status_flags = dynamic_cache.bmd_status;
    bmd_return.capability_flags = static_cache.bmd_capability;
    bmd_return.recalibrate_count = static_cache.bmd_recalibrate_count;
    bmd_return.quick_recalibrate_time = static_cache.bmd_quick_recalibrate_time;
    bmd_return.slow_recalibrate_time = static_cache.bmd_slow_recalibrate_time;
}

pub(crate) fn compute_bct(
    payload: &embedded_batteries_async::acpi::Bct,
    bct_return: &mut embedded_batteries_async::acpi::BctReturnResult,
    _dynamic_cache: &DynamicBatteryMsgs,
) {
    // Just echo back charge level for now
    // TODO: Actually compute time from charge level
    *bct_return = embedded_batteries_async::acpi::BctReturnResult::from(payload.charge_level_percent);
}

pub(crate) fn compute_btm(
    payload: &embedded_batteries_async::acpi::Btm,
    btm_return: &mut embedded_batteries_async::acpi::BtmReturnResult,
    _dynamic_cache: &DynamicBatteryMsgs,
) {
    // Just echo back charge level for now
    // TODO: Actually compute time from charge level
    *btm_return = embedded_batteries_async::acpi::BtmReturnResult::from(payload.discharge_rate);
}

pub(crate) fn compute_sta(sta_return: &mut embedded_batteries_async::acpi::StaReturn) {
    // TODO: Grab real state values
    *sta_return = embedded_batteries_async::acpi::StaReturn::all();
}

pub(crate) fn compute_psr(psr_return: &mut embedded_batteries_async::acpi::PsrReturn, psu_state: &PsuState) {
    // TODO: Refactor to check if battery if force discharged,
    // which should give an offline result even when the PSU is attached.
    psr_return.power_source = if psu_state.psu_connected {
        embedded_batteries_async::acpi::PowerSource::Online
    } else {
        embedded_batteries_async::acpi::PowerSource::Offline
    };
}

pub(crate) fn compute_pif<'a>(psu_state: &PsuState) -> Pif<'a> {
    // TODO: Grab real values from power policy
    let capability = psu_state.power_capability.unwrap_or(PowerCapability {
        voltage_mv: 0,
        current_ma: 0,
    });

    Pif {
        power_source_state: PowerSourceState::empty(),
        max_output_power: capability.max_power_mw(),
        max_input_power: capability.max_power_mw(),
        model_number: &[],
        serial_number: &[],
        oem_info: &[],
    }
}

impl<'a> crate::context::Context<'a> {
    async fn send_acpi_response(&self, payload: &crate::acpi::Payload<'_>) {
        let acpi_response: AcpiMsgComms;

        {
            let mut buf_access = self.get_acpi_buf_owned_ref().borrow_mut();

            if let Ok(payload_len) = payload.to_raw(buf_access.borrow_mut()) {
                acpi_response = AcpiMsgComms {
                    payload: crate::context::acpi_buf::get(),
                    payload_len,
                };
            } else {
                error!("payload to_raw error, sending empty response");
                acpi_response = AcpiMsgComms {
                    payload: crate::context::acpi_buf::get(),
                    payload_len: 0,
                };
            }
        }

        super::comms_send(
            crate::EndpointID::External(embedded_services::comms::External::Host),
            &HostMsg::Response(acpi_response),
        )
        .await
        .unwrap();

        debug!("response sent to espi_service");
    }

    pub(super) async fn bix_handler(&self, fg: &Device, payload: &crate::acpi::Payload<'_>) {
        trace!("Battery service: got BIX command!");
        // Enough space for all string fields to have 7 bytes + 1 null terminator byte
        let mut bix_data = [0u8; 100];
        let static_cache_guard = fg.get_static_battery_cache_guarded().await;
        let dynamic_cache_guard = fg.get_dynamic_battery_cache_guarded().await;
        let bix_return = compute_bix(static_cache_guard.deref(), dynamic_cache_guard.deref());

        let model_num_size = bix_return.model_number.len();
        let serial_num_size = bix_return.serial_number.len();
        let battery_type_size = bix_return.battery_type.len();
        let oem_info_size = bix_return.oem_info.len();

        bix_return
            .to_bytes(
                &mut bix_data,
                model_num_size,
                serial_num_size,
                battery_type_size,
                oem_info_size,
            )
            .unwrap_or_else(|_| error!("Computing BIX return failed!"));

        // Drop locks before next await point to eliminate possibility of deadlock
        drop(static_cache_guard);
        drop(dynamic_cache_guard);
        let response = crate::acpi::Payload {
            version: 1,
            instance: 1,
            reserved: 0,
            command: payload.command,
            data: &bix_data,
        };

        self.send_acpi_response(&response).await;
    }

    pub(super) async fn bst_handler(&self, fg: &Device, payload: &crate::acpi::Payload<'_>) {
        trace!("Battery service: got BST command!");
        let cache = fg.get_dynamic_battery_cache().await;
        let mut bst_data = embedded_batteries_async::acpi::BstReturn::default();
        compute_bst(&mut bst_data, &cache);
        let bst_data: &[u8; BST_RETURN_SIZE_BYTES] = zerocopy::transmute_ref!(&bst_data);
        let response = crate::acpi::Payload {
            version: 1,
            instance: 1,
            reserved: 0,
            command: payload.command,
            data: bst_data,
        };
        self.send_acpi_response(&response).await;
    }

    pub(super) async fn psr_handler(&self, _fg: &Device, payload: &crate::acpi::Payload<'_>) {
        trace!("Battery service: got PSR command!");

        let mut psr_data = PsrReturn::default();

        compute_psr(&mut psr_data, &self.get_power_info().await);

        let psr_data: &[u8; PSR_RETURN_SIZE_BYTES] = zerocopy::transmute_ref!(&psr_data);

        let response = crate::acpi::Payload {
            version: 1,
            instance: 1,
            reserved: 0,
            command: payload.command,
            data: psr_data,
        };

        self.send_acpi_response(&response).await;
    }

    pub(super) async fn pif_handler(&self, _fg: &Device, payload: &crate::acpi::Payload<'_>) {
        trace!("Battery service: got PIF command!");
        // Enough space for all string fields to have 7 bytes + 1 null terminator byte
        let mut pif_data = [0u8; 36];
        let pif_return = compute_pif(&self.get_power_info().await);

        let model_num_size = pif_return.model_number.len();
        let serial_num_size = pif_return.serial_number.len();
        let oem_info_size = pif_return.oem_info.len();
        pif_return
            .to_bytes(&mut pif_data, model_num_size, serial_num_size, oem_info_size)
            .unwrap_or_else(|_| error!("Computing PIF return failed!"));

        let response = crate::acpi::Payload {
            version: 1,
            instance: 1,
            reserved: 0,
            command: payload.command,
            data: &pif_data,
        };

        self.send_acpi_response(&response).await;
    }

    pub(super) async fn bps_handler(&self, fg: &Device, payload: &crate::acpi::Payload<'_>) {
        trace!("Battery service: got BPS command!");

        let mut bps_data = embedded_batteries_async::acpi::Bps::default();

        let cache = fg.get_dynamic_battery_cache().await;
        compute_bps(&mut bps_data, &cache);
        let bps_data: &[u8; BPS_RETURN_SIZE_BYTES] = zerocopy::transmute_ref!(&bps_data);

        let response = crate::acpi::Payload {
            version: 1,
            instance: 1,
            reserved: 0,
            command: payload.command,
            data: bps_data,
        };
        self.send_acpi_response(&response).await;
    }

    pub(super) async fn btp_handler(&self, _fg: &Device, payload: &crate::acpi::Payload<'_>) {
        trace!("Battery service: got BTP command!");

        // TODO: Save trip point

        // 0 for success, 1 for failure
        let ret_status = 0u8;

        let response = crate::acpi::Payload {
            version: 1,
            instance: 1,
            reserved: ret_status,
            command: payload.command,
            data: &[],
        };

        self.send_acpi_response(&response).await;
    }

    pub(super) async fn bpt_handler(&self, _fg: &Device, payload: &crate::acpi::Payload<'_>) {
        trace!("Battery service: got BPT command!");

        // 0 for success, 1 for failure
        let mut ret_status = 1u8;

        if payload.data.len() >= 12 {
            // TODO: Save power threshold somewhere
            // Safe from panics as length is verified above.
            let threshold_id = u32::from_le_bytes(payload.data[4..8].try_into().unwrap());
            let threshold_value = u32::from_le_bytes(payload.data[8..12].try_into().unwrap());
            info!("Threshold ID: {}, Threshold value: {}", threshold_id, threshold_value);
            ret_status = 0;
        } else {
            error!("Malformed BPT command")
        }

        let response = crate::acpi::Payload {
            version: 1,
            instance: 1,
            reserved: ret_status,
            command: payload.command,
            data: &[],
        };

        self.send_acpi_response(&response).await;
    }

    pub(super) async fn bpc_handler(&self, fg: &Device, payload: &crate::acpi::Payload<'_>) {
        trace!("Battery service: got BPC command!");

        let mut bpc_data = embedded_batteries_async::acpi::Bpc::default();

        let cache = fg.get_static_battery_cache().await;
        compute_bpc(&mut bpc_data, &cache);
        let bpc_data: &[u8; BPC_RETURN_SIZE_BYTES] = zerocopy::transmute_ref!(&bpc_data);

        let response = crate::acpi::Payload {
            version: 1,
            instance: 1,
            reserved: 0,
            command: payload.command,
            data: bpc_data,
        };

        self.send_acpi_response(&response).await;
    }

    pub(super) async fn bmc_handler(&self, _fg: &Device, payload: &crate::acpi::Payload<'_>) {
        trace!("Battery service: got BMC command!");

        // 0 for success, 1 for failure
        let mut ret_status = 1u8;

        if payload.data.len() >= 4 {
            // TODO: Save power threshold somewhere
            // Safe from panics as length is verified above.
            let raw_bmc_control_flags =
                BmcControlFlags::from_bits_truncate(u32::from_le_bytes(payload.data[..4].try_into().unwrap()));
            info!("Recvd BMC flags: {}", raw_bmc_control_flags.bits());
            ret_status = 0;
        } else {
            error!("Malformed BMC command")
        }

        let response = crate::acpi::Payload {
            version: 1,
            instance: 1,
            reserved: ret_status,
            command: payload.command,
            data: &[],
        };

        self.send_acpi_response(&response).await;
    }

    pub(super) async fn bmd_handler(&self, fg: &Device, payload: &crate::acpi::Payload<'_>) {
        trace!("Battery service: got BMD command!");
        let mut bmd_data = embedded_batteries_async::acpi::Bmd::default();
        let static_cache = fg.get_static_battery_cache().await;
        let dynamic_cache = fg.get_dynamic_battery_cache().await;
        compute_bmd(&mut bmd_data, &static_cache, &dynamic_cache);
        let bmd_data: &[u8; BMD_RETURN_SIZE_BYTES] = zerocopy::transmute_ref!(&bmd_data);

        let response = crate::acpi::Payload {
            version: 1,
            instance: 1,
            reserved: 0,
            command: payload.command,
            data: bmd_data,
        };

        self.send_acpi_response(&response).await;
    }

    pub(super) async fn bct_handler(&self, fg: &Device, payload: &crate::acpi::Payload<'_>) {
        trace!("Battery service: got BCT command!");

        let mut ret_status = 1;
        let mut bct_data = BctReturnResult::default();

        if payload.data.len() >= 4 {
            // TODO: Save power threshold somewhere
            // Safe from panics as length is verified above.
            let raw_bct = Bct {
                charge_level_percent: u32::from_le_bytes(payload.data[..4].try_into().unwrap()),
            };
            info!("Recvd BCT charge_level_percent: {}", raw_bct.charge_level_percent);
            compute_bct(&raw_bct, &mut bct_data, &fg.get_dynamic_battery_cache().await);
            ret_status = 0;
        } else {
            error!("Malformed BCT command")
        }

        let bct_return: &[u8; BCT_RETURN_SIZE_BYTES] = &bct_data.into();

        let response = crate::acpi::Payload {
            version: 1,
            instance: 1,
            reserved: ret_status,
            command: payload.command,
            data: bct_return,
        };

        self.send_acpi_response(&response).await;
    }

    pub(super) async fn btm_handler(&self, fg: &Device, payload: &crate::acpi::Payload<'_>) {
        trace!("Battery service: got BTM command!");

        let mut ret_status = 1;
        let mut btm_data = BtmReturnResult::default();

        if payload.data.len() >= 4 {
            // TODO: Save power threshold somewhere
            // Safe from panics as length is verified above.
            let raw_btm = Btm {
                discharge_rate: u32::from_le_bytes(payload.data[..4].try_into().unwrap()),
            };
            info!("Recvd BTM discharge_rate: {}", raw_btm.discharge_rate);
            compute_btm(&raw_btm, &mut btm_data, &fg.get_dynamic_battery_cache().await);
            ret_status = 0;
        } else {
            error!("Malformed BTM command")
        }

        let btm_return: &[u8; BTM_RETURN_SIZE_BYTES] = &btm_data.into();

        let response = crate::acpi::Payload {
            version: 1,
            instance: 1,
            reserved: ret_status,
            command: payload.command,
            data: btm_return,
        };

        self.send_acpi_response(&response).await;
    }

    pub(super) async fn bms_handler(&self, _fg: &Device, payload: &crate::acpi::Payload<'_>) {
        trace!("Battery service: got BMS command!");

        let mut ret_status = 1;

        if payload.data.len() >= 4 {
            // TODO: Set sampling time
            // Safe from panics as length is verified above.
            let sampling_time = u32::from_le_bytes(payload.data[..4].try_into().unwrap());
            info!("Recvd BMS sampling_time: {}", sampling_time);
            ret_status = 0;
        } else {
            error!("Malformed BMS command")
        }

        let response = crate::acpi::Payload {
            version: 1,
            instance: 1,
            reserved: ret_status,
            command: payload.command,
            // Weirdly, BMS is a method with a dedicated result status in the data field.
            // We use the reserved field for our own return value, so just mirror it here.
            data: &u32::to_le_bytes(u32::from(ret_status)),
        };

        self.send_acpi_response(&response).await;
    }

    pub(super) async fn bma_handler(&self, _fg: &Device, payload: &crate::acpi::Payload<'_>) {
        trace!("Battery service: got BMA command!");

        let mut ret_status = 1;

        if payload.data.len() >= 4 {
            // TODO: Save power threshold somewhere
            // Safe from panics as length is verified above.
            let raw_bma = Bma {
                averaging_interval_ms: u32::from_le_bytes(payload.data[..4].try_into().unwrap()),
            };
            info!("Recvd BMA averaging_interval_ms: {}", raw_bma.averaging_interval_ms);
            ret_status = 0;
        } else {
            error!("Malformed BMA command")
        }

        let response = crate::acpi::Payload {
            version: 1,
            instance: 1,
            reserved: ret_status,
            command: payload.command,
            // Weirdly, BMA is a method with a dedicated result status in the data field.
            // We use the reserved field for our own return value, so just mirror it here.
            data: &u32::to_le_bytes(u32::from(ret_status)),
        };

        self.send_acpi_response(&response).await;
    }

    pub(super) async fn sta_handler(&self, _fg: &Device, payload: &crate::acpi::Payload<'_>) {
        trace!("Battery service: got STA command!");

        let mut sta_data = embedded_batteries_async::acpi::StaReturn::default();

        compute_sta(&mut sta_data);
        let sta_data: &[u8; STA_RETURN_SIZE_BYTES] = zerocopy::transmute_ref!(&sta_data);

        let response = crate::acpi::Payload {
            version: 1,
            instance: 1,
            reserved: 0,
            command: payload.command,
            data: sta_data,
        };

        self.send_acpi_response(&response).await;
    }
}
