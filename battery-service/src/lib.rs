#![no_std]

use battery_service_interface::{
    BatteryError, Bct, BctReturnResult, BixFixedStrings, Bma, Bmc, Bmd, Bms, Bpc, Bps, Bpt, BstReturn, Btm,
    BtmReturnResult, Btp, PifFixedStrings, PsrReturn, StaReturn,
};
use core::marker::PhantomData;
use embedded_services::info;

mod acpi;
#[cfg(feature = "mock")]
pub mod mock;
pub mod registration;

pub use registration::{ArrayRegistration, Registration};

// Re-export the fuel gauge interface so that OEM drivers and integrators can
// implement and use the battery service without depending on the interface crate directly.
pub use battery_service_interface::fuel_gauge::{
    DynamicBatteryMsgs, FuelGauge, FuelGaugeError, InternalState, OperationalSubstate, PresentSubstate, State,
    StaticBatteryMsgs,
};
pub use battery_service_interface::{BatteryService, DeviceId};

/// The battery service.
///
/// Owns the [`Registration`] that provides the set of fuel gauges, and answers
/// ACPI battery queries (via the [`BatteryService`] trait) by reading each
/// registered fuel gauge's cached state. The OEM drives each registered fuel
/// gauge directly through the [`FuelGauge`] trait methods.
pub struct Service<'hw, Reg: Registration<'hw>> {
    pub registration: Reg,
    _phantom: PhantomData<&'hw ()>,
}

impl<'hw, Reg: Registration<'hw>> Service<'hw, Reg> {
    /// Create a new battery service that owns the provided registration.
    pub fn new(registration: Reg) -> Self {
        info!("Starting battery-service");
        Self {
            registration,
            _phantom: PhantomData,
        }
    }
}

impl<'hw, Reg: Registration<'hw>> battery_service_interface::BatteryService for Service<'hw, Reg> {
    async fn battery_charge_time(
        &self,
        battery_id: DeviceId,
        charge_level: Bct,
    ) -> Result<BctReturnResult, BatteryError> {
        self.bct_handler(battery_id, charge_level).await
    }

    async fn battery_info(&self, battery_id: DeviceId) -> Result<BixFixedStrings, BatteryError> {
        self.bix_handler(battery_id).await
    }

    async fn set_battery_measurement_averaging_interval(
        &self,
        battery_id: DeviceId,
        bma: Bma,
    ) -> Result<(), BatteryError> {
        self.bma_handler(battery_id, bma).await
    }

    async fn battery_maintenance_control(&self, battery_id: DeviceId, bmc: Bmc) -> Result<(), BatteryError> {
        self.bmc_handler(battery_id, bmc).await
    }

    async fn battery_maintenance_data(&self, battery_id: DeviceId) -> Result<Bmd, BatteryError> {
        self.bmd_handler(battery_id).await
    }

    async fn set_battery_measurement_sampling_time(
        &self,
        battery_id: DeviceId,
        battery_measurement_sampling: Bms,
    ) -> Result<(), BatteryError> {
        self.bms_handler(battery_id, battery_measurement_sampling).await
    }

    async fn battery_power_characteristics(&self, battery_id: DeviceId) -> Result<Bpc, BatteryError> {
        self.bpc_handler(battery_id).await
    }

    async fn battery_power_state(&self, battery_id: DeviceId) -> Result<Bps, BatteryError> {
        self.bps_handler(battery_id).await
    }

    async fn set_battery_power_threshold(
        &self,
        battery_id: DeviceId,
        power_threshold: Bpt,
    ) -> Result<(), BatteryError> {
        self.bpt_handler(battery_id, power_threshold).await
    }

    async fn battery_status(&self, battery_id: DeviceId) -> Result<BstReturn, BatteryError> {
        self.bst_handler(battery_id).await
    }

    async fn battery_time_to_empty(
        &self,
        battery_id: DeviceId,
        battery_discharge_rate: Btm,
    ) -> Result<BtmReturnResult, BatteryError> {
        self.btm_handler(battery_id, battery_discharge_rate).await
    }

    async fn set_battery_trip_point(&self, battery_id: DeviceId, btp: Btp) -> Result<(), BatteryError> {
        self.btp_handler(battery_id, btp).await
    }

    async fn is_psu_in_use(&self, battery_id: DeviceId) -> Result<PsrReturn, BatteryError> {
        self.psr_handler(battery_id).await
    }

    async fn power_source_information(&self, power_source_id: DeviceId) -> Result<PifFixedStrings, BatteryError> {
        self.pif_handler(power_source_id).await
    }

    async fn device_status(&self, battery_id: DeviceId) -> Result<StaReturn, BatteryError> {
        self.sta_handler(battery_id).await
    }
}
