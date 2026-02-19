use embassy_time::{Duration, Timer};
use embedded_batteries_async::{
    acpi, charger,
    smart_battery::{self, SmartBattery},
};
use embedded_services::{GlobalRawMutex, error, info};

// Convenience fns
pub async fn init_state_machine(battery_service: &'static crate::Service) -> Result<(), crate::context::ContextError> {
    battery_service
        .execute_event(crate::context::BatteryEvent {
            event: crate::context::BatteryEventInner::DoInit,
            device_id: crate::device::DeviceId(0),
        })
        .await
        .inspect_err(|f| embedded_services::debug!("Fuel gauge init error: {:?}", f))?;

    battery_service
        .execute_event(crate::context::BatteryEvent {
            event: crate::context::BatteryEventInner::PollStaticData,
            device_id: crate::device::DeviceId(0),
        })
        .await
        .inspect_err(|f| embedded_services::debug!("Fuel gauge static data error: {:?}", f))?;

    battery_service
        .execute_event(crate::context::BatteryEvent {
            event: crate::context::BatteryEventInner::PollDynamicData,
            device_id: crate::device::DeviceId(0),
        })
        .await
        .inspect_err(|f| embedded_services::debug!("Fuel gauge dynamic data error: {:?}", f))?;

    Ok(())
}

pub async fn recover_state_machine(battery_service: &'static crate::Service) -> Result<(), ()> {
    loop {
        match battery_service
            .execute_event(crate::context::BatteryEvent {
                event: crate::context::BatteryEventInner::Timeout,
                device_id: crate::device::DeviceId(0),
            })
            .await
        {
            Ok(_) => {
                embedded_services::info!("FG recovered!");
                return Ok(());
            }
            Err(e) => match e {
                crate::context::ContextError::StateError(e) => match e {
                    crate::context::StateMachineError::DeviceTimeout => {
                        embedded_services::trace!("Recovery failed, trying again after a backoff period");
                        Timer::after(Duration::from_secs(10)).await;
                    }
                    crate::context::StateMachineError::NoOpRecoveryFailed => {
                        embedded_services::error!("Couldn't recover, reinit needed");
                        return Err(());
                    }
                    _ => embedded_services::debug!("Unexpected error"),
                },
                _ => embedded_services::debug!("Unexpected error"),
            },
        }
    }
}

pub type MockBattery<'a> = crate::wrapper::Wrapper<'a, MockBatteryDriver>;

#[derive(Default)]
pub struct MockBatteryDriver {
    capacity_mode_bit: embassy_sync::mutex::Mutex<GlobalRawMutex, bool>,
}

impl MockBatteryDriver {
    pub fn new() -> Self {
        MockBatteryDriver {
            capacity_mode_bit: embassy_sync::mutex::Mutex::new(false),
        }
    }

    async fn set_capacity_bit(&mut self, mwh: bool) -> Result<(), MockBatteryError> {
        let battery_mode = self.battery_mode().await?;
        SmartBattery::set_battery_mode(self, battery_mode.with_capacity_mode(mwh)).await?;
        *self.capacity_mode_bit.get_mut() = mwh;

        Ok(())
    }
}

#[derive(Clone, Copy, Debug)]
pub struct MockBatteryError;

impl crate::controller::Controller for MockBatteryDriver {
    type ControllerError = MockBatteryError;

    async fn initialize(&mut self) -> Result<(), Self::ControllerError> {
        // Milliamps
        let mwh = false;
        self.set_capacity_bit(mwh)
            .await
            .inspect_err(|_| error!("FG: failed to initialize"))?;

        info!("FG: initialized");
        Ok(())
    }

    async fn ping(&mut self) -> Result<(), Self::ControllerError> {
        if let Err(e) = self.charging_voltage().await {
            error!("FG: failed to ping");
            Err(e)
        } else {
            info!("FG: ping success");
            Ok(())
        }
    }

    async fn get_dynamic_data(&mut self) -> Result<crate::device::DynamicBatteryMsgs, Self::ControllerError> {
        let new_msgs = crate::device::DynamicBatteryMsgs {
            average_current_ma: self.average_current().await?,
            battery_status: self.battery_status().await?.into(),
            max_power_mw: 100,
            battery_temp_dk: self.temperature().await?,
            sus_power_mw: 42,
            charging_current_ma: self.charging_current().await?,
            charging_voltage_mv: self.charging_voltage().await?,
            voltage_mv: self.voltage().await?,
            current_ma: self.current().await?,
            full_charge_capacity_mwh: match self.full_charge_capacity().await? {
                smart_battery::CapacityModeValue::CentiWattUnsigned(_) => 0xDEADBEEF,
                smart_battery::CapacityModeValue::MilliAmpUnsigned(capacity) => capacity.into(),
            },
            remaining_capacity_mwh: match self.remaining_capacity().await? {
                smart_battery::CapacityModeValue::CentiWattUnsigned(_) => 0xDEADBEEF,
                smart_battery::CapacityModeValue::MilliAmpUnsigned(capacity) => capacity.into(),
            },
            relative_soc_pct: self.relative_state_of_charge().await?.into(),
            cycle_count: self.cycle_count().await?,
            max_error_pct: self.max_error().await?.into(),
            bmd_status: acpi::BmdStatusFlags::default(),
            turbo_vload_mv: 0,
            turbo_rhf_effective_mohm: 0,
        };
        Ok(new_msgs)
    }

    #[allow(clippy::indexing_slicing)]
    async fn get_static_data(&mut self) -> Result<crate::device::StaticBatteryMsgs, Self::ControllerError> {
        let design_capacity: u32 = match self.design_capacity().await? {
            smart_battery::CapacityModeValue::CentiWattUnsigned(design_capacity) => design_capacity.into(),
            smart_battery::CapacityModeValue::MilliAmpUnsigned(design_capacity) => design_capacity.into(),
        };

        let mut new_msgs = crate::device::StaticBatteryMsgs {
            manufacturer_name: Default::default(),
            device_name: Default::default(),
            device_chemistry: Default::default(),
            design_capacity_mwh: match self.design_capacity().await? {
                smart_battery::CapacityModeValue::CentiWattUnsigned(design_capacity) => design_capacity.into(),
                smart_battery::CapacityModeValue::MilliAmpUnsigned(design_capacity) => design_capacity.into(),
            },
            design_voltage_mv: self.design_voltage().await?,
            device_chemistry_id: Default::default(),
            serial_num: Default::default(),
            battery_mode: self.battery_mode().await?,
            design_cap_warning: design_capacity / 4,
            design_cap_low: design_capacity / 10,
            measurement_accuracy: self.max_error().await?.into(),
            max_sample_time: Default::default(),
            min_sample_time: Default::default(),
            max_averaging_interval: Default::default(),
            min_averaging_interval: Default::default(),
            cap_granularity_1: Default::default(),
            cap_granularity_2: Default::default(),
            power_threshold_support: battery_service_messages::PowerThresholdSupport::empty(),
            max_instant_pwr_threshold: Default::default(),
            max_sus_pwr_threshold: Default::default(),
            bmc_flags: battery_service_messages::BmcControlFlags::empty(),
            bmd_capability: battery_service_messages::BmdCapabilityFlags::empty(),
            bmd_recalibrate_count: Default::default(),
            bmd_quick_recalibrate_time: Default::default(),
            bmd_slow_recalibrate_time: Default::default(),
        };
        let mut buf = [0u8; 21];

        let buf_len = new_msgs.manufacturer_name.len();
        self.manufacturer_name(&mut buf[..buf_len]).await?;
        new_msgs.manufacturer_name.copy_from_slice(&buf[..buf_len]);

        let buf_len = new_msgs.device_name.len();
        self.device_name(&mut buf[..buf_len]).await?;
        new_msgs.device_name.copy_from_slice(&buf[..buf_len]);

        let buf_len = new_msgs.device_chemistry.len();
        self.device_chemistry(&mut buf[..buf_len]).await?;
        new_msgs.device_chemistry.copy_from_slice(&buf[..buf_len]);

        let buf_len = new_msgs.device_chemistry_id.len();
        self.device_chemistry(&mut buf[..buf_len]).await?;
        new_msgs.device_chemistry_id.copy_from_slice(&buf[..buf_len]);

        let serial = self.serial_number().await?;
        let serial = serial.to_le_bytes();
        new_msgs.serial_num = [serial[0], serial[1], 0, 0];

        Ok(new_msgs)
    }

    async fn get_device_event(&mut self) -> crate::controller::ControllerEvent {
        // TODO: Loop forever till we figure out what we want to do here
        loop {
            Timer::after_secs(1000000).await;
        }
    }

    fn set_timeout(&mut self, _duration: embassy_time::Duration) {}
}

impl smart_battery::Error for MockBatteryError {
    fn kind(&self) -> smart_battery::ErrorKind {
        smart_battery::ErrorKind::Other
    }
}

impl smart_battery::ErrorType for MockBatteryDriver {
    type Error = MockBatteryError;
}

// Revisit: Have this generate realistic data dynamically (right now just static arbitrary values)
impl smart_battery::SmartBattery for MockBatteryDriver {
    async fn absolute_state_of_charge(&mut self) -> Result<smart_battery::Percent, Self::Error> {
        Ok(77)
    }

    async fn at_rate(&mut self) -> Result<smart_battery::CapacityModeSignedValue, Self::Error> {
        Ok(smart_battery::CapacityModeSignedValue::MilliAmpSigned(100))
    }

    async fn at_rate_ok(&mut self) -> Result<bool, Self::Error> {
        Ok(true)
    }

    async fn at_rate_time_to_empty(&mut self) -> Result<smart_battery::Minutes, Self::Error> {
        Ok(2600)
    }

    async fn at_rate_time_to_full(&mut self) -> Result<smart_battery::Minutes, Self::Error> {
        Ok(1337)
    }

    async fn average_current(&mut self) -> Result<smart_battery::MilliAmpsSigned, Self::Error> {
        Ok(42)
    }

    async fn average_time_to_empty(&mut self) -> Result<smart_battery::Minutes, Self::Error> {
        Ok(100)
    }

    async fn average_time_to_full(&mut self) -> Result<smart_battery::Minutes, Self::Error> {
        Ok(120)
    }

    async fn battery_mode(&mut self) -> Result<smart_battery::BatteryModeFields, Self::Error> {
        Ok(smart_battery::BatteryModeFields::new())
    }

    async fn battery_status(&mut self) -> Result<smart_battery::BatteryStatusFields, Self::Error> {
        Ok(smart_battery::BatteryStatusFields::new())
    }

    async fn charging_current(&mut self) -> Result<charger::MilliAmps, Self::Error> {
        Ok(50)
    }

    async fn charging_voltage(&mut self) -> Result<charger::MilliVolts, Self::Error> {
        Ok(4242)
    }

    async fn current(&mut self) -> Result<smart_battery::MilliAmpsSigned, Self::Error> {
        Ok(500)
    }

    async fn cycle_count(&mut self) -> Result<smart_battery::Cycles, Self::Error> {
        Ok(10000)
    }

    async fn design_capacity(&mut self) -> Result<smart_battery::CapacityModeValue, Self::Error> {
        Ok(smart_battery::CapacityModeValue::CentiWattUnsigned(0))
    }

    async fn design_voltage(&mut self) -> Result<charger::MilliVolts, Self::Error> {
        Ok(12000)
    }

    #[allow(clippy::indexing_slicing)]
    async fn device_chemistry(&mut self, chemistry: &mut [u8]) -> Result<(), Self::Error> {
        let bytes = [b'L', b'i', b'P', b'o', 0];
        let bytes_to_copy = core::cmp::min(bytes.len(), chemistry.len());
        chemistry[..bytes_to_copy].copy_from_slice(&bytes[..bytes_to_copy]);
        Ok(())
    }

    #[allow(clippy::indexing_slicing)]
    async fn device_name(&mut self, name: &mut [u8]) -> Result<(), Self::Error> {
        let bytes = [b'O', b'd', b'p', b'B', b'a', b't', b't', 0];
        let bytes_to_copy = core::cmp::min(bytes.len(), name.len());
        name[..bytes_to_copy].copy_from_slice(&bytes[..bytes_to_copy]);
        Ok(())
    }

    async fn full_charge_capacity(&mut self) -> Result<smart_battery::CapacityModeValue, Self::Error> {
        Ok(smart_battery::CapacityModeValue::CentiWattUnsigned(0))
    }

    async fn manufacture_date(&mut self) -> Result<smart_battery::ManufactureDate, Self::Error> {
        Ok(smart_battery::ManufactureDate::new())
    }

    #[allow(clippy::indexing_slicing)]
    async fn manufacturer_name(&mut self, name: &mut [u8]) -> Result<(), Self::Error> {
        let bytes = [b'B', b'a', b't', b'B', b'r', b'o', b's', 0];
        let bytes_to_copy = core::cmp::min(bytes.len(), name.len());
        name[..bytes_to_copy].copy_from_slice(&bytes[..bytes_to_copy]);
        Ok(())
    }

    async fn max_error(&mut self) -> Result<smart_battery::Percent, Self::Error> {
        Ok(2)
    }

    async fn relative_state_of_charge(&mut self) -> Result<smart_battery::Percent, Self::Error> {
        Ok(10)
    }

    async fn remaining_capacity(&mut self) -> Result<smart_battery::CapacityModeValue, Self::Error> {
        Ok(smart_battery::CapacityModeValue::CentiWattUnsigned(0))
    }

    async fn remaining_capacity_alarm(&mut self) -> Result<smart_battery::CapacityModeValue, Self::Error> {
        Ok(smart_battery::CapacityModeValue::CentiWattUnsigned(0))
    }

    async fn remaining_time_alarm(&mut self) -> Result<smart_battery::Minutes, Self::Error> {
        Ok(85)
    }

    async fn run_time_to_empty(&mut self) -> Result<smart_battery::Minutes, Self::Error> {
        Ok(110)
    }

    async fn serial_number(&mut self) -> Result<u16, Self::Error> {
        Ok(0x4544)
    }

    async fn set_at_rate(&mut self, _rate: smart_battery::CapacityModeSignedValue) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn set_battery_mode(&mut self, _flags: smart_battery::BatteryModeFields) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn set_remaining_capacity_alarm(
        &mut self,
        _capacity: smart_battery::CapacityModeValue,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn set_remaining_time_alarm(&mut self, _time: smart_battery::Minutes) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn specification_info(&mut self) -> Result<smart_battery::SpecificationInfoFields, Self::Error> {
        Ok(smart_battery::SpecificationInfoFields::new())
    }

    async fn temperature(&mut self) -> Result<smart_battery::DeciKelvin, Self::Error> {
        Ok(2981)
    }

    async fn voltage(&mut self) -> Result<smart_battery::MilliVolts, Self::Error> {
        Ok(12600)
    }
}
