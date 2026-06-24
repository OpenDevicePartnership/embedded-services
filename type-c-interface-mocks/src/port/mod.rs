//! Mock implementation of the per-port [`type_c_interface::port::pd::Pd`] and
//! [`power_policy_interface::psu::Psu`] traits.

use embedded_services::error;
use embedded_services::event::NonBlockingSender;
use embedded_services::named::Named;
use embedded_usb_pd::vdm::structured::command::discover_identity::{sop, sop_prime};
use embedded_usb_pd::{PdError, PlugOrientation, PowerRole, ado::Ado, type_c::ConnectionState};
use power_policy_interface::capability::{
    ConsumerFlags, ConsumerPowerCapability, PowerCapability, ProviderPowerCapability, PsuType,
};
use power_policy_interface::psu::{Error as PsuError, Psu, State};
use type_c_interface::control::{
    dp::{DpConfig, DpStatus},
    pd::PortStatus,
    svid::DiscoveredSvids,
    tbt::TbtConfig,
    usb::UsbControlConfig,
    vdm::{AttnVdm, OtherVdm, SendVdm},
};
use type_c_interface::port::event::PortStatusEventBitfield;
use type_c_interface::port::pd::Pd;
use type_c_interface::service::event::StatusChangedData;

/// Error type for [`PortMock`] control operations
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum PortMockError {
    /// A disconnect was requested but the port was not connected
    NotConnected,
}

/// Additional connection parameters for [`PortMock::connect`]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct ConnectionConfig {
    /// Port partner supports dual-power roles
    pub dual_power: bool,
    /// Plug orientation
    pub plug_orientation: PlugOrientation,
}

impl Default for ConnectionConfig {
    fn default() -> Self {
        Self {
            dual_power: false,
            plug_orientation: PlugOrientation::CC1,
        }
    }
}

/// Mock implementation of a single Type-C port for use in tests
pub struct PortMock<
    TypeCSender: NonBlockingSender<type_c_interface::service::event::PortEventData>,
    PowerSender: NonBlockingSender<power_policy_interface::psu::event::EventData>,
> {
    name: &'static str,
    /// Current port status returned by [`Pd::get_port_status`]
    status: PortStatus,
    /// Current power-policy PSU state
    psu_state: State,
    /// Type-C event sender
    type_c_sender: TypeCSender,
    /// Power policy event sender
    power_sender: PowerSender,
}

impl<TypeCSender, PowerSender> PortMock<TypeCSender, PowerSender>
where
    TypeCSender: NonBlockingSender<type_c_interface::service::event::PortEventData>,
    PowerSender: NonBlockingSender<power_policy_interface::psu::event::EventData>,
{
    /// Create a new mock with the given name
    pub fn new(name: &'static str, type_c_sender: TypeCSender, power_sender: PowerSender) -> Self {
        Self {
            name,
            status: PortStatus::new(),
            psu_state: State::default(),
            type_c_sender,
            power_sender,
        }
    }

    /// Returns the current port status
    pub fn status(&self) -> &PortStatus {
        &self.status
    }

    /// Simulate a connection on this port.
    ///
    /// Rebuilds the stored [`PortStatus`] to reflect an attached connection in
    /// the given [`PowerRole`] with the provided [`PowerCapability`].
    pub fn connect(
        &mut self,
        role: PowerRole,
        capability: PowerCapability,
        config: ConnectionConfig,
    ) -> Result<(), PsuError> {
        let mut status = PortStatus::new();
        status.connection_state = Some(ConnectionState::Attached);
        status.dual_power = config.dual_power;
        status.plug_orientation = config.plug_orientation;
        status.power_role = role;
        self.psu_state.attach()?;

        // Notify services
        if self
            .power_sender
            .try_send(power_policy_interface::psu::event::EventData::Attached)
            .is_none()
        {
            error!("Failed to send attached to power policy");
        }

        let mut status_event = PortStatusEventBitfield::none();
        status_event.set_plug_inserted_or_removed(true);

        match role {
            PowerRole::Source => {
                status_event.set_new_power_contract_as_consumer(true);
                status_event.set_sink_ready(true);
                status.available_source_contract = Some(capability);
                self.psu_state
                    .update_consumer_power_capability(Some(ConsumerPowerCapability {
                        capability,
                        flags: ConsumerFlags::none().with_psu_type(PsuType::TypeC),
                    }))?;
            }
            PowerRole::Sink => status.available_sink_contract = Some(capability),
        }

        let previous_status = self.status;
        self.status = status;

        if self
            .type_c_sender
            .try_send(type_c_interface::service::event::PortEventData::StatusChanged(
                StatusChangedData {
                    status_event,
                    previous_status,
                    current_status: self.status,
                },
            ))
            .is_none()
        {
            error!("Failed to send Type-C status changed event");
        }
        Ok(())
    }

    /// Simulate a disconnection on this port.
    ///
    /// Returns [`PortMockError::NotConnected`] if the port was not connected.
    pub fn disconnect(&mut self) -> Result<(), PortMockError> {
        if !self.status.is_connected() {
            return Err(PortMockError::NotConnected);
        }
        self.psu_state.detach();

        // Notify services
        if self
            .power_sender
            .try_send(power_policy_interface::psu::event::EventData::Detached)
            .is_none()
        {
            error!("Failed to send detached to power policy");
        }

        let mut status_event = PortStatusEventBitfield::none();
        status_event.set_plug_inserted_or_removed(true);

        let previous_status = self.status;
        self.status = PortStatus::default();

        if self
            .type_c_sender
            .try_send(type_c_interface::service::event::PortEventData::StatusChanged(
                StatusChangedData {
                    status_event,
                    previous_status,
                    current_status: self.status,
                },
            ))
            .is_none()
        {
            error!("Failed to send Type-C status changed event");
        }
        Ok(())
    }
}

impl<TypeCSender, PowerSender> Named for PortMock<TypeCSender, PowerSender>
where
    TypeCSender: NonBlockingSender<type_c_interface::service::event::PortEventData>,
    PowerSender: NonBlockingSender<power_policy_interface::psu::event::EventData>,
{
    fn name(&self) -> &'static str {
        self.name
    }
}

impl<TypeCSender, PowerSender> Pd for PortMock<TypeCSender, PowerSender>
where
    TypeCSender: NonBlockingSender<type_c_interface::service::event::PortEventData>,
    PowerSender: NonBlockingSender<power_policy_interface::psu::event::EventData>,
{
    async fn get_port_status(&mut self) -> Result<PortStatus, PdError> {
        Ok(self.status)
    }

    async fn clear_dead_battery_flag(&mut self) -> Result<(), PdError> {
        Ok(())
    }

    async fn enable_sink_path(&mut self, _enable: bool) -> Result<(), PdError> {
        Ok(())
    }

    async fn get_pd_alert(&mut self) -> Result<Option<Ado>, PdError> {
        Ok(None)
    }

    async fn set_unconstrained_power(&mut self, _unconstrained: bool) -> Result<(), PdError> {
        Ok(())
    }

    async fn get_other_vdm(&mut self) -> Result<OtherVdm, PdError> {
        Ok(OtherVdm::default())
    }

    async fn get_attn_vdm(&mut self) -> Result<AttnVdm, PdError> {
        Ok(AttnVdm::default())
    }

    async fn send_vdm(&mut self, _tx_vdm: SendVdm) -> Result<(), PdError> {
        Ok(())
    }

    async fn execute_drst(&mut self) -> Result<(), PdError> {
        Ok(())
    }

    async fn hard_reset(&mut self) -> Result<(), PdError> {
        Ok(())
    }

    async fn get_dp_status(&mut self) -> Result<DpStatus, PdError> {
        Ok(DpStatus::default())
    }

    async fn set_dp_config(&mut self, _config: DpConfig) -> Result<(), PdError> {
        Ok(())
    }

    async fn set_tbt_config(&mut self, _config: TbtConfig) -> Result<(), PdError> {
        Ok(())
    }

    async fn set_usb_control(&mut self, _config: UsbControlConfig) -> Result<(), PdError> {
        Ok(())
    }

    async fn get_discovered_svids(&mut self) -> Result<DiscoveredSvids, PdError> {
        Ok(DiscoveredSvids::default())
    }

    async fn get_discover_identity_sop_response(&mut self) -> Result<sop::ResponseVdos, PdError> {
        Err(PdError::Failed)
    }

    async fn get_discover_identity_sop_prime_response(&mut self) -> Result<sop_prime::ResponseVdos, PdError> {
        Err(PdError::Failed)
    }
}

impl<TypeCSender, PowerSender> Psu for PortMock<TypeCSender, PowerSender>
where
    TypeCSender: NonBlockingSender<type_c_interface::service::event::PortEventData>,
    PowerSender: NonBlockingSender<power_policy_interface::psu::event::EventData>,
{
    async fn disconnect(&mut self) -> Result<(), PsuError> {
        self.enable_sink_path(false).await.map_err(|_| PsuError::Failed)?;
        self.psu_state.detach();
        Ok(())
    }

    async fn connect_provider(&mut self, _capability: ProviderPowerCapability) -> Result<(), PsuError> {
        Ok(())
    }

    async fn connect_consumer(&mut self, capability: ConsumerPowerCapability) -> Result<(), PsuError> {
        self.enable_sink_path(true).await.map_err(|_| PsuError::Failed)?;
        self.psu_state.connect_consumer(capability)
    }

    fn state(&self) -> &State {
        &self.psu_state
    }

    fn state_mut(&mut self) -> &mut State {
        &mut self.psu_state
    }
}
