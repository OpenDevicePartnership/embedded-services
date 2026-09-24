use core::marker::PhantomData;
use core::ptr;

use embedded_services::named::Named as _;
use embedded_services::sync::Lockable;
use embedded_services::{debug, error, info, trace};
use embedded_usb_pd::GlobalPortId;
use embedded_usb_pd::PdError as Error;
use embedded_usb_pd::ado::Ado;
use power_policy_interface::service::event::EventData as PowerPolicyEventData;
use tcpm_interface::control::dp::DpStatus;
use tcpm_interface::control::pd::PortStatus;
use tcpm_interface::port::event::VdmData;
use tcpm_interface::port::notification::NotificationHandler;
use tcpm_interface::port::pd::Pd;
use tcpm_interface::service::event::{PortEvent, PortEventData};
use tcpm_interface::service::notification::Notifier as _;

use tcpm_interface::port::event::PortStatusEventBitfield;

use crate::service::registration::Registration;

pub mod config;
pub mod event_receiver;
mod power;
pub mod registration;
mod ucsi;

/// Type-C service
///
/// Constructing a Service is the first step in using the Type-C service.
/// Arguments should be an initialized context
pub struct Service<'port, Reg: Registration<'port>> {
    /// UCSI state
    ucsi: ucsi::State,
    /// Config
    config: config::Config,
    /// Service registration
    registration: Reg,
    _phantom: PhantomData<&'port ()>,
}

/// Type-C service events
#[derive(Clone)]
pub enum Event<'port, Port: Lockable<Inner: Pd>> {
    /// Port event
    PortEvent(PortEvent<'port, Port>),
    /// Power policy event
    PowerPolicy(PowerPolicyEventData),
}

impl<'port, Reg: Registration<'port>> Service<'port, Reg> {
    /// Create a new service the given configuration
    pub fn new(config: config::Config, registration: Reg) -> Self {
        Self {
            ucsi: ucsi::State::default(),
            config,
            registration,
            _phantom: PhantomData,
        }
    }

    fn get_port_index(&self, port: &'port Reg::Port) -> Result<usize, Error> {
        self.registration
            .ports()
            .iter()
            .position(|p| ptr::eq(*p, port))
            .ok_or(Error::InvalidPort)
    }

    /// Look up the port for a given global port ID
    fn lookup_port(&self, port_id: GlobalPortId) -> Result<&'port Reg::Port, Error> {
        self.registration
            .ports()
            .get(port_id.0 as usize)
            .ok_or(Error::InvalidPort)
            .copied()
    }

    /// Send an event to all registered listeners
    async fn notify_debug_accessory(&mut self, port: &'port Reg::Port, connected: bool) {
        for notifier in self.registration.notifiers() {
            if let Err(e) = notifier.notify_debug_accessory(port, connected).await {
                error!("Failed to notify debug accessory: {:#?}", e);
            }
        }
    }

    async fn notify_ucsi_change_indicator(&mut self, port: &'port Reg::Port, port_id: GlobalPortId, notify_opm: bool) {
        for notifier in self.registration.notifiers() {
            if let Err(e) = notifier.notify_ucsi_change_indicator(port, port_id, notify_opm).await {
                error!("Failed to notify UCSI change indicator: {:#?}", e);
            }
        }
    }

    /// Process events for a specific port
    async fn process_port_status_event(
        &mut self,
        port: &'port Reg::Port,
        event: PortStatusEventBitfield,
        new_status: PortStatus,
        old_status: PortStatus,
    ) -> Result<(), Error> {
        let port_name = { port.lock().await.name() };

        debug!("({}): Event: {:#?}", port_name, event);
        debug!("({}) Previous status: {:#?}", port_name, old_status);
        debug!("({}) Status: {:#?}", port_name, new_status);

        let connection_changed = new_status.is_connected() != old_status.is_connected();
        if connection_changed && (new_status.is_debug_accessory() || old_status.is_debug_accessory()) {
            // Notify that a debug connection has connected/disconnected
            if new_status.is_connected() {
                debug!("({}): Debug accessory connected", port_name);
            } else {
                debug!("({}): Debug accessory disconnected", port_name);
            }

            self.notify_debug_accessory(port, new_status.is_connected()).await;
        }

        self.handle_ucsi_port_event(port, GlobalPortId(self.get_port_index(port)? as u8), event, &new_status)
            .await;

        Ok(())
    }

    async fn process_port_event(&mut self, event: &PortEvent<'port, Reg::Port>) -> Result<(), Error> {
        match event.event {
            PortEventData::StatusChanged(data) => {
                self.process_notify_status_changed(
                    event.port,
                    data.status_event,
                    data.previous_status,
                    data.current_status,
                )
                .await
            }
            PortEventData::Alert(ado) => self.process_notify_alert(event.port, ado).await,
            PortEventData::Vdm(vdm) => self.process_notify_vdm(event.port, vdm).await,
            PortEventData::DiscoverModeCompleted => self.process_notify_discover_mode_completed(event.port).await,
            PortEventData::UsbMuxErrorRecovery => self.process_notify_usb_mux_error_recovery(event.port).await,
            PortEventData::DpStatusUpdate(status) => self.process_notify_dp_status_update(event.port, status).await,
            e => {
                debug!(
                    "({}): Received unhandled port event: {:#?}",
                    event.port.lock().await.name(),
                    e
                );
                Ok(())
            }
        }
    }

    /// Process the given event
    pub async fn process_event(&mut self, event: Event<'port, Reg::Port>) -> Result<(), Error> {
        match event {
            Event::PortEvent(event) => {
                trace!("({}): Processing port event", event.port.lock().await.name());
                self.process_port_event(&event).await
            }
            Event::PowerPolicy(event) => {
                trace!("Processing power policy event");
                self.process_power_policy_event(&event).await
            }
        }
    }
}

impl<'port, Reg: Registration<'port>> NotificationHandler<'port> for Service<'port, Reg> {
    type Port = Reg::Port;

    async fn process_notify_status_changed(
        &mut self,
        port: &'port Reg::Port,
        status_event: PortStatusEventBitfield,
        previous_status: PortStatus,
        current_status: PortStatus,
    ) -> Result<(), Error> {
        self.process_port_status_event(port, status_event, current_status, previous_status)
            .await
    }

    async fn process_notify_alert(&mut self, port: &'port Reg::Port, alert: Ado) -> Result<(), Error> {
        // Currently just log notifications, but may want to do more in the future
        debug!("({}): Received PD alert: {:#?}", port.lock().await.name(), alert);
        Ok(())
    }

    async fn process_notify_vdm(&mut self, port: &'port Reg::Port, vdm: VdmData) -> Result<(), Error> {
        debug!("({}): Received VDM: {:#?}", port.lock().await.name(), vdm);
        Ok(())
    }

    async fn process_notify_discover_mode_completed(&mut self, port: &'port Reg::Port) -> Result<(), Error> {
        debug!("({}): Discover mode completed", port.lock().await.name());
        Ok(())
    }

    async fn process_notify_usb_mux_error_recovery(&mut self, port: &'port Reg::Port) -> Result<(), Error> {
        debug!("({}): USB mux error recovery", port.lock().await.name());
        Ok(())
    }

    async fn process_notify_dp_status_update(&mut self, port: &'port Reg::Port, status: DpStatus) -> Result<(), Error> {
        debug!("({}): DP status update: {:#?}", port.lock().await.name(), status);
        Ok(())
    }
}
