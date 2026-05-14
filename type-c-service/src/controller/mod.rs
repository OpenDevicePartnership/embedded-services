//! Struct that manages per-port state, interfacing with a controller object that exposes multiple ports.
use embedded_services::{debug, error, event::Sender, info, named::Named, sync::Lockable};
use embedded_usb_pd::{LocalPortId, PdError};
use power_policy_interface::psu::PsuState;
use type_c_interface::control::pd::PortStatus;
use type_c_interface::controller::pd::Pd;
use type_c_interface::port::event::PortEventBitfield;
use type_c_interface::port::{event::PortEvent as InterfacePortEvent, event::PortStatusEventBitfield};
use type_c_interface::service::event::{PortEventData as ServicePortEventData, StatusChangedData};

use crate::controller::event::{Event, Loopback};
use crate::controller::state::SharedState;

pub mod config;
pub mod electrical_disconnect;
pub mod event;
pub mod event_receiver;
pub mod macros;
pub mod max_sink_voltage;
mod pd;
mod power;
pub mod retimer;
pub mod state;
pub mod type_c;
pub mod ucsi;

pub struct Port<
    'device,
    C: Lockable<Inner: Pd>,
    Shared: Lockable<Inner = SharedState>,
    TypeCSender: Sender<type_c_interface::service::event::PortEventData>,
    PowerSender: Sender<power_policy_interface::psu::event::EventData>,
    LoopbackSender: Sender<event::Loopback>,
> {
    /// Local port
    port: LocalPortId,
    /// Controller
    controller: &'device C,
    /// Per-port PSU state
    psu_state: power_policy_interface::psu::State,
    /// Name for this port
    name: &'static str,
    /// Cached port status
    status: PortStatus,
    /// Sender for type-c service events
    type_c_sender: TypeCSender,
    /// Sender for power policy events
    power_policy_sender: PowerSender,
    /// Configuration
    config: config::Config,
    /// Shared state
    shared_state: &'device Shared,
    /// Loopback sender
    loopback_sender: LoopbackSender,
}

impl<
    'device,
    C: Lockable<Inner: Pd>,
    Shared: Lockable<Inner = SharedState>,
    TypeCSender: Sender<type_c_interface::service::event::PortEventData>,
    PowerSender: Sender<power_policy_interface::psu::event::EventData>,
    LoopbackSender: Sender<event::Loopback>,
> Port<'device, C, Shared, TypeCSender, PowerSender, LoopbackSender>
{
    /// Create new Port instance
    // TODO: refactor arguments into a registration struct
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        name: &'static str,
        config: config::Config,
        port: LocalPortId,
        controller: &'device C,
        shared_state: &'device Shared,
        type_c_sender: TypeCSender,
        power_policy_sender: PowerSender,
        loopback_sender: LoopbackSender,
    ) -> Self {
        Self {
            name,
            controller,
            port,
            status: PortStatus::default(),
            psu_state: power_policy_interface::psu::State::default(),
            power_policy_sender,
            config,
            shared_state,
            loopback_sender,
            type_c_sender,
        }
    }

    /// Top-level processing function
    pub async fn process_event(&mut self, event: Event) -> Result<Option<ServicePortEventData>, PdError> {
        match event {
            Event::PortEvent(port_event) => self.process_port_event(port_event).await,
        }
    }

    /// Process a port notification
    async fn process_port_event(&mut self, event: InterfacePortEvent) -> Result<Option<ServicePortEventData>, PdError> {
        match event {
            InterfacePortEvent::StatusChanged(status_event) => {
                self.process_port_status_changed(status_event).await.map(Some)
            }
            InterfacePortEvent::Alert => self.process_pd_alert().await,
            InterfacePortEvent::Vdm(vdm_event) => self.process_vdm_event(vdm_event).await.map(Some),
            InterfacePortEvent::DpStatusUpdate => self.process_dp_status_update().await.map(Some),
            rest => {
                // Nothing currently implemented for these
                debug!("({}): Notification: {:#?}", self.name, rest);
                Ok(None)
            }
        }
    }

    /// Process port status changed events
    async fn process_port_status_changed(
        &mut self,
        status_event: PortStatusEventBitfield,
    ) -> Result<ServicePortEventData, PdError> {
        let new_status = self.controller.lock().await.get_port_status(self.port).await?;
        debug!("({}) status: {:#?}", self.name, new_status);
        debug!("({}) status events: {:#?}", self.name, status_event);

        if status_event.plug_inserted_or_removed() {
            self.process_plug_event(&new_status).await?;
        }

        // Only notify power policy of a contract after Sink Ready event (always after explicit or implicit contract)
        if status_event.sink_ready() {
            self.process_new_consumer_contract(&new_status).await?;
        }

        if status_event.new_power_contract_as_provider() {
            self.process_new_provider_contract(&new_status).await?;
        }

        self.check_sink_ready_timeout(
            &new_status,
            status_event.new_power_contract_as_consumer(),
            status_event.sink_ready(),
        )
        .await?;

        let event = ServicePortEventData::StatusChanged(StatusChangedData {
            status_event,
            previous_status: self.status,
            current_status: new_status,
        });
        self.status = new_status;
        self.type_c_sender.send(event).await;
        Ok(event)
    }

    /// Handle a plug event
    async fn process_plug_event(&mut self, new_status: &PortStatus) -> Result<(), PdError> {
        info!("Plug event");
        if new_status.is_connected() {
            info!("Plug inserted");
            if self.psu_state.psu_state != PsuState::Detached {
                info!("Device not in detached state, recovering");
                self.psu_state.detach();
            }

            if let Err(e) = self.psu_state.attach() {
                // This should never happen because we should have detached above
                error!("Failed to attach PSU: {:?}", e);
                return Err(PdError::Failed);
            }

            self.power_policy_sender
                .send(power_policy_interface::psu::event::EventData::Attached)
                .await;
        } else {
            info!("Plug removed");
            self.psu_state.detach();
            self.power_policy_sender
                .send(power_policy_interface::psu::event::EventData::Detached)
                .await;
        }

        Ok(())
    }

    /// Get the cached port status, returns None if the port is invalid
    pub fn get_cached_port_status(&self) -> PortStatus {
        self.status
    }

    /// Synchronize the state between the controller and the internal state
    pub async fn sync_state(&mut self) -> Result<(), PdError> {
        let status = self.controller.lock().await.get_port_status(self.port).await?;

        let mut event = PortEventBitfield::none();
        let previous_status = self.status;

        if previous_status.is_connected() != status.is_connected() {
            event.status.set_plug_inserted_or_removed(true);
        }

        if previous_status.available_sink_contract != status.available_sink_contract {
            event.status.set_new_power_contract_as_consumer(true);
        }

        if previous_status.available_source_contract != status.available_source_contract {
            event.status.set_new_power_contract_as_provider(true);
        }

        if event != PortEventBitfield::none() {
            self.loopback_sender.send(Loopback::PortEvent(event)).await;
        }
        Ok(())
    }
}

impl<
    'device,
    C: Lockable<Inner: Pd>,
    Shared: Lockable<Inner = SharedState>,
    TypeCSender: Sender<type_c_interface::service::event::PortEventData>,
    PowerSender: Sender<power_policy_interface::psu::event::EventData>,
    LoopbackSender: Sender<event::Loopback>,
> Named for Port<'device, C, Shared, TypeCSender, PowerSender, LoopbackSender>
{
    fn name(&self) -> &'static str {
        self.name
    }
}
