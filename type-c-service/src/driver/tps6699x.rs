use core::array::from_fn;
use core::cell::{Cell, RefCell};
use core::iter::zip;

use ::tps6699x::registers::field_sets::IntEventBus1;
use ::tps6699x::registers::{PdCcPullUp, PlugMode};
use ::tps6699x::{TPS66993_NUM_PORTS, TPS66994_NUM_PORTS};
use embassy_futures::select::select;
use embassy_sync::blocking_mutex::raw::RawMutex;
use embassy_sync::signal::Signal;
use embedded_hal_async::i2c::I2c;
use embedded_services::power::policy::{self, PowerCapability};
use embedded_services::type_c::controller::{self, Controller, PortStatus};
use embedded_services::type_c::event::PortEventKind;
use embedded_services::type_c::ControllerId;
use embedded_services::{debug, info, trace, type_c};
use embedded_usb_pd::pdo::{sink, source, Common, Rdo};
use embedded_usb_pd::type_c::Current as TypecCurrent;
use embedded_usb_pd::{Error, GlobalPortId, PdError, PortId as LocalPortId, PowerRole};
use tps6699x::asynchronous::embassy as tps6699x;

use crate::wrapper::ControllerWrapper;

pub struct Tps6699x<'a, const N: usize, M: RawMutex, B: I2c> {
    port_events: [Cell<PortEventKind>; N],
    port_status: [Cell<PortStatus>; N],
    sw_event: Signal<M, ()>,
    tps6699x: RefCell<tps6699x::Tps6699x<'a, M, B>>,
}

impl<'a, const N: usize, M: RawMutex, B: I2c> Tps6699x<'a, N, M, B> {
    fn new(tps6699x: tps6699x::Tps6699x<'a, M, B>) -> Self {
        Self {
            port_events: [const { Cell::new(PortEventKind::none()) }; N],
            port_status: [const { Cell::new(PortStatus::new()) }; N],
            sw_event: Signal::new(),
            tps6699x: RefCell::new(tps6699x),
        }
    }

    /// Reads and caches the current status of the port, returns any detected events
    async fn update_port_status(
        &self,
        tps6699x: &mut tps6699x::Tps6699x<'a, M, B>,
        port: LocalPortId,
    ) -> Result<PortEventKind, Error<B::Error>> {
        #[allow(unused_mut)]
        let mut events = PortEventKind::none();

        let status = tps6699x.get_port_status(port).await?;
        trace!("Port{} status: {:#?}", port.0, status);

        let pd_status = tps6699x.get_pd_status(port).await?;
        trace!("Port{} PD status: {:#?}", port.0, pd_status);

        let port_control = tps6699x.get_port_control(port).await?;
        trace!("Port{} control: {:#?}", port.0, port_control);

        let mut port_status = PortStatus::default();

        let plug_present = status.plug_present();
        let valid_connection = matches!(
            status.connection_state(),
            PlugMode::Audio | PlugMode::Debug | PlugMode::ConnectedNoRa | PlugMode::Connected
        );

        debug!("Port{} Plug present: {}", port.0, plug_present);
        debug!("Port{} Valid connection: {}", port.0, valid_connection);

        port_status.connection_present = plug_present && valid_connection;

        if port_status.connection_present {
            port_status.debug_connection = status.connection_state() == PlugMode::Debug;

            // Determine current contract if any
            let pdo_raw = tps6699x.get_active_pdo_contract(port).await?.active_pdo();
            info!("Raw PDO: {:#X}", pdo_raw);
            let rdo_raw = tps6699x.get_active_rdo_contract(port).await?.active_rdo();
            info!("Raw RDO: {:#X}", rdo_raw);

            if pdo_raw != 0 && rdo_raw != 0 {
                // Got a valid explicit contract
                if pd_status.is_source() {
                    let pdo = source::Pdo::try_from(pdo_raw).map_err(Error::Pd)?;
                    let rdo = Rdo::for_pdo(rdo_raw, pdo);
                    debug!("PDO: {:#?}", pdo);
                    debug!("RDO: {:#?}", rdo);
                    port_status.available_source_contract = Some(PowerCapability::from(pdo));
                    port_status.dual_power = pdo.is_dual_role();
                } else {
                    let pdo = sink::Pdo::try_from(pdo_raw).map_err(Error::Pd)?;
                    let rdo = Rdo::for_pdo(rdo_raw, pdo);
                    debug!("PDO: {:#?}", pdo);
                    debug!("RDO: {:#?}", rdo);
                    port_status.available_sink_contract = Some(PowerCapability::from(pdo));
                    port_status.dual_power = pdo.is_dual_role()
                }
            } else if pd_status.is_source() {
                // Implicit source contract
                let current = TypecCurrent::try_from(port_control.typec_current()).map_err(Error::Pd)?;
                debug!("Port{} type-C source current: {:#?}", port.0, current);
                let new_contract = Some(PowerCapability::from(current));

                if new_contract != port_status.available_source_contract {
                    debug!("New implicit contract as provider");
                    // We don't get interrupts for implicit contracts so generate event manually
                    events.set_new_power_contract_as_provider(true);
                }

                port_status.available_source_contract = new_contract;
            } else {
                // Implicit sink contract
                let pull = pd_status.cc_pull_up();
                let new_contract = if pull == PdCcPullUp::NoPull {
                    // No pull up means no contract
                    debug!("Port{} no pull up", port.0);
                    None
                } else {
                    let current = TypecCurrent::try_from(pd_status.cc_pull_up()).map_err(Error::Pd)?;
                    debug!("Port{} type-C sink current: {:#?}", port.0, current);
                    Some(PowerCapability::from(current))
                };

                if new_contract.is_some() && new_contract != port_status.available_sink_contract {
                    debug!("New implicit contract as consumer");
                    // We don't get interrupts for implicit contracts so generate event manually
                    events.set_new_power_contract_as_consumer(true);
                }

                port_status.available_sink_contract = new_contract;
            }
        }

        self.port_status[port.0 as usize].set(port_status);
        Ok(events)
    }

    /// Wait for an event on any port
    async fn wait_interrupt_event(&self, tps6699x: &mut tps6699x::Tps6699x<'a, M, B>) -> Result<(), Error<B::Error>> {
        let interrupts = tps6699x.wait_interrupt(false, |_, _| true).await;

        for (interrupt, cell) in zip(interrupts.iter(), self.port_events.iter()) {
            if *interrupt == IntEventBus1::new_zero() {
                continue;
            }

            let mut event = cell.get();
            if interrupt.plug_event() {
                debug!("Plug event");
                event.set_plug_inserted_or_removed(true);
            }

            if interrupt.new_consumer_contract() {
                debug!("New consumer contract");
                event.set_new_power_contract_as_consumer(true);
            }

            if interrupt.new_provider_contract() {
                debug!("New consumer contract");
                event.set_new_power_contract_as_provider(true);
            }

            cell.set(event);
        }
        Ok(())
    }

    /// Wait for a software event
    async fn wait_sw_event(&self) {
        self.sw_event.wait().await;
    }

    /// Signal an event on the given port
    #[allow(dead_code)]
    fn signal_event(&self, port: LocalPortId, event: PortEventKind) {
        if port.0 >= self.port_events.len() as u8 {
            return;
        }

        let cell = &self.port_events[port.0 as usize];
        let current = cell.get();
        cell.set(current.union(event));
        self.sw_event.signal(());
    }
}

impl<const N: usize, M: RawMutex, B: I2c> Controller for Tps6699x<'_, N, M, B> {
    type BusError = B::Error;

    /// Wait for an event on any port
    #[allow(clippy::await_holding_refcell_ref)]
    async fn wait_port_event(&mut self) -> Result<(), Error<Self::BusError>> {
        let mut tps6699x = self.tps6699x.borrow_mut();
        let _ = select(self.wait_interrupt_event(&mut tps6699x), self.wait_sw_event()).await;

        for (i, cell) in self.port_events.iter().enumerate() {
            let port = LocalPortId(i as u8);

            let event = cell.get().union(self.update_port_status(&mut tps6699x, port).await?);

            // TODO: We get interrupts for certain status changes that don't currently map to a generic port event
            // Enable this when those get fleshed out
            // Ignore empty events
            /*if event == PortEventKind::NONE {
                continue;
            }*/

            trace!("Port{} event: {:#?}", i, event);
            cell.set(event);
        }
        Ok(())
    }

    /// Returns and clears current events for the given port
    async fn clear_port_events(&mut self, port: LocalPortId) -> Result<PortEventKind, Error<Self::BusError>> {
        if port.0 >= self.port_events.len() as u8 {
            return PdError::InvalidPort.into();
        }

        Ok(self.port_events[port.0 as usize].replace(PortEventKind::none()))
    }

    /// Returns the current status of the port
    async fn get_port_status(
        &mut self,
        port: LocalPortId,
    ) -> Result<type_c::controller::PortStatus, Error<Self::BusError>> {
        if port.0 >= self.port_status.len() as u8 {
            return PdError::InvalidPort.into();
        }

        Ok(self.port_status[port.0 as usize].get())
    }

    #[allow(clippy::await_holding_refcell_ref)]
    async fn enable_sink_path(&mut self, port: LocalPortId, enable: bool) -> Result<(), Error<Self::BusError>> {
        debug!("Port{} enable sink path: {}", port.0, enable);
        let mut tps6699x = self.tps6699x.borrow_mut();
        tps6699x.enable_sink_path(port, enable).await
    }

    #[allow(clippy::await_holding_refcell_ref)]
    async fn set_sourcing(&mut self, port: LocalPortId, enable: bool) -> Result<(), Error<Self::BusError>> {
        debug!("Port{} enable source: {}", port.0, enable);
        let mut tps6699x = self.tps6699x.borrow_mut();
        tps6699x.enable_source(port, enable).await
    }

    #[allow(clippy::await_holding_refcell_ref)]
    async fn set_source_current(
        &mut self,
        port: LocalPortId,
        current: TypecCurrent,
        signal_event: bool,
    ) -> Result<(), Error<Self::BusError>> {
        debug!("Port{} set source current: {:?}", port.0, current);

        let mut tps6699x = self.tps6699x.borrow_mut();
        let mut port_control = tps6699x.get_port_control(port).await?;
        port_control.set_typec_current(current.into());

        tps6699x.set_port_control(port, port_control).await?;
        if signal_event {
            let mut event = PortEventKind::none();
            event.set_new_power_contract_as_consumer(true);
            self.signal_event(port, event);
        }
        Ok(())
    }

    #[allow(clippy::await_holding_refcell_ref)]
    async fn request_pr_swap(
        &mut self,
        port: LocalPortId,
        role: embedded_usb_pd::PowerRole,
    ) -> Result<(), Error<Self::BusError>> {
        debug!("Port{} request PR swap to {:?}", port.0, role);

        let mut tps6699x = self.tps6699x.borrow_mut();
        let mut control = tps6699x.get_port_control(port).await?;
        match role {
            PowerRole::Sink => control.set_initiate_swap_to_sink(true),
            PowerRole::Source => control.set_initiate_swap_to_source(true),
        }

        tps6699x.set_port_control(port, control).await
    }
}

/// TPS66994 controller wrapper
pub type Tps66994Wrapper<'a, M, B> = ControllerWrapper<'a, TPS66994_NUM_PORTS, Tps6699x<'a, TPS66994_NUM_PORTS, M, B>>;

/// TPS66993 controller wrapper
pub type Tps66993Wrapper<'a, M, B> = ControllerWrapper<'a, TPS66993_NUM_PORTS, Tps6699x<'a, TPS66993_NUM_PORTS, M, B>>;

/// Create a TPS66994 controller wrapper
pub fn tps66994<'a, M: RawMutex, B: I2c>(
    controller: tps6699x::Tps6699x<'a, M, B>,
    controller_id: ControllerId,
    port_ids: &'a [GlobalPortId],
    power_ids: [policy::DeviceId; TPS66994_NUM_PORTS],
) -> Result<Tps66994Wrapper<'a, M, B>, PdError> {
    if port_ids.len() != TPS66994_NUM_PORTS {
        return Err(PdError::InvalidParams);
    }

    Ok(ControllerWrapper::new(
        controller::Device::new(controller_id, port_ids),
        from_fn(|i| policy::device::Device::new(power_ids[i])),
        Tps6699x::new(controller),
    ))
}

/// Create a new TPS66993 controller wrapper
pub fn tps66993<'a, M: RawMutex, B: I2c>(
    controller: tps6699x::Tps6699x<'a, M, B>,
    controller_id: ControllerId,
    port_ids: &'a [GlobalPortId],
    power_ids: [policy::DeviceId; TPS66993_NUM_PORTS],
) -> Result<Tps66993Wrapper<'a, M, B>, PdError> {
    if port_ids.len() != TPS66993_NUM_PORTS {
        return Err(PdError::InvalidParams);
    }

    Ok(ControllerWrapper::new(
        controller::Device::new(controller_id, port_ids),
        from_fn(|i| policy::device::Device::new(power_ids[i])),
        Tps6699x::new(controller),
    ))
}
