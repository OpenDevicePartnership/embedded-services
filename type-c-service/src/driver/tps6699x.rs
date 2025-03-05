use core::array::from_fn;
use core::cell::RefCell;
use core::iter::zip;

use ::tps6699x::registers::field_sets::IntEventBus1;
use ::tps6699x::registers::{PdCcPullUp, PlugMode};
use ::tps6699x::{TPS66993_NUM_PORTS, TPS66994_NUM_PORTS};
use embassy_futures::select::{select, select_array, Either};
use embassy_sync::blocking_mutex::raw::RawMutex;
use embassy_sync::signal::Signal;
use embedded_hal_async::i2c::I2c;
use embedded_services::power::policy::{self, PowerCapability};
use embedded_services::type_c::controller::{self, Contract, PortStatus, MAX_CONTROLLER_PORTS};
use embedded_services::type_c::event::PortEventKind;
use embedded_services::type_c::{ControllerId, GlobalPortId};
use embedded_services::{debug, info, trace, type_c};
use embedded_usb_pd::pdo::Common;
use embedded_usb_pd::pdo::{sink, source, Rdo};
use embedded_usb_pd::type_c::Current as TypecCurrent;
use embedded_usb_pd::{Error, PdError, PortId as LocalPortId, PowerRole};
use tps6699x::asynchronous::embassy as tps6699x;

use crate::wrapper::{Controller, ControllerWrapper};

pub struct Tps6699x<'a, M: RawMutex, B: I2c> {
    port_events: [PortEventKind; MAX_CONTROLLER_PORTS],
    port_status: [PortStatus; MAX_CONTROLLER_PORTS],
    port_sw_events: [Signal<M, PortEventKind>; MAX_CONTROLLER_PORTS],
    tps6699x: RefCell<tps6699x::Tps6699x<'a, M, B>>,
}

impl<'a, M: RawMutex, B: I2c> Tps6699x<'a, M, B> {
    fn new(tps6699x: tps6699x::Tps6699x<'a, M, B>) -> Self {
        Self {
            port_events: [PortEventKind::NONE; MAX_CONTROLLER_PORTS],
            port_status: [PortStatus::default(); MAX_CONTROLLER_PORTS],
            port_sw_events: [const { Signal::new() }; MAX_CONTROLLER_PORTS],
            tps6699x: RefCell::new(tps6699x),
        }
    }
}

impl<'a, M: RawMutex, B: I2c> Tps6699x<'a, M, B> {
    /// Reads and caches port status, returns any detected events
    async fn update_port_status(&mut self, port: LocalPortId) -> Result<PortEventKind, Error<B::Error>> {
        let mut events = PortEventKind::NONE;
        let mut tps6699x = self.tps6699x.borrow_mut();

        if port.0 >= self.port_status.len() as u8 {
            return PdError::InvalidPort.into();
        }

        let status = tps6699x.get_port_status(port).await?;
        trace!("Port{} status: {:#?}", port.0, status);

        let pd_status = tps6699x.get_pd_status(port).await?;
        trace!("Port{} PD status: {:#?}", port.0, pd_status);

        let port_control = tps6699x.get_port_control(port).await?;
        trace!("Port{} control: {:#?}", port.0, port_control);

        let mut port_status = PortStatus::default();

        let plug_present = status.plug_present();
        let valid_connection = match status.connection_state() {
            PlugMode::Audio | PlugMode::Debug | PlugMode::ConnectedNoRa | PlugMode::Connected => true,
            _ => false,
        };

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
                    debug!("Source PDO: {:#?}", pdo);
                    debug!("Source RDO: {:#?}", rdo);
                    port_status.contract = Some(Contract::from(pdo));
                    port_status.dual_power = pdo.is_dual_role();
                } else {
                    let pdo = sink::Pdo::try_from(pdo_raw).map_err(Error::Pd)?;
                    let rdo = Rdo::for_pdo(rdo_raw, pdo);
                    debug!("Sink PDO: {:#?}", pdo);
                    debug!("Sink RDO: {:#?}", rdo);
                    port_status.contract = Some(Contract::from(pdo));
                    port_status.dual_power = pdo.is_dual_role();
                }
            } else {
                if pd_status.is_source() {
                    // Implicit source contract
                    let current = TypecCurrent::try_from(port_control.typec_current()).map_err(Error::Pd)?;
                    debug!("Port{} type-C source current: {:#?}", port.0, current);
                    let new_contract = Some(Contract::Source(PowerCapability::from(current)));

                    if !new_contract.is_none() && new_contract != port_status.contract {
                        debug!("New implicit contract as provider");
                        // We don't get interrupts for implicit contracts so generate event manually
                        events |= PortEventKind::NEW_POWER_CONTRACT_AS_PROVIDER;
                    }

                    port_status.contract = new_contract;
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
                        Some(Contract::Sink(PowerCapability::from(current)))
                    };

                    if !new_contract.is_none() && new_contract != port_status.contract {
                        debug!("New implicit contract as consumer");
                        // We don't get interrupts for implicit contracts so generate event manually
                        events |= PortEventKind::NEW_POWER_CONTRACT_AS_CONSUMER;
                    }

                    port_status.contract = new_contract;
                }
            }
        }

        self.port_status[port.0 as usize] = port_status;
        Ok(events)
    }

    async fn wait_interrupt_event(&self) -> Result<[PortEventKind; MAX_CONTROLLER_PORTS], Error<B::Error>> {
        let mut tps6699x = self.tps6699x.borrow_mut();
        let interrupts = tps6699x.wait_interrupt(false, |_, _| true).await;
        let mut events = [PortEventKind::NONE; MAX_CONTROLLER_PORTS];

        for (i, (interrupt, event)) in zip(interrupts.iter(), events.iter_mut()).enumerate() {
            trace!("Port{} interrupt: {:#?}", i, interrupt);

            if *interrupt == IntEventBus1::new_zero() {
                continue;
            }

            if interrupt.plug_event() {
                debug!("Plug event");
                *event |= PortEventKind::PLUG_INSERTED_OR_REMOVED;
            }

            if interrupt.new_consumer_contract() {
                debug!("New consumer contract");
                *event |= PortEventKind::NEW_POWER_CONTRACT_AS_CONSUMER;
            }

            if interrupt.new_provider_contract() {
                debug!("New provider contract");
                *event |= PortEventKind::NEW_POWER_CONTRACT_AS_PROVIDER;
            }
        }

        Ok(events)
    }

    async fn wait_sw_event(&self) -> Result<[PortEventKind; MAX_CONTROLLER_PORTS], Error<B::Error>> {
        let futures: [_; MAX_CONTROLLER_PORTS] = from_fn(async |i| self.port_sw_events[i].wait().await);
        let sw_event = select_array(futures).await;

        let mut events = [PortEventKind::NONE; MAX_CONTROLLER_PORTS];
        events[sw_event.1] = sw_event.0;
        trace!("Port{} SW event: {:#?}", sw_event.0, events[sw_event.1]);
        Ok(events)
    }

    fn signal_sw_event(&self, port: LocalPortId, event: PortEventKind) {
        if port.0 >= self.port_sw_events.len() as u8 {
            return;
        }

        self.port_sw_events[port.0 as usize].signal(event);
    }
}

impl<'a, M: RawMutex, B: I2c> Controller for Tps6699x<'a, M, B> {
    type BusError = B::Error;

    /// Wait for an event on any port
    async fn wait_port_event(&mut self) -> Result<(), Error<Self::BusError>> {
        let events = match select(self.wait_interrupt_event(), self.wait_sw_event()).await {
            Either::First(r) => r,
            Either::Second(r) => r,
        }?;

        for (i, mut event) in events.into_iter().enumerate() {
            trace!("Port{} event: {:#?}", i, event);
            event |= self.port_events[i];

            // TODO: We get interrupts for certain status changes that don't currently map to a generic port event
            // Enable this when those get fleshed out
            // Ignore empty events
            /*if event == PortEventKind::NONE {
                continue;
            }*/
            let port = LocalPortId(i as u8);
            event |= self.update_port_status(port).await?;
            self.port_events[i] = event;
        }
        Ok(())
    }

    /// Returns and clears current events for the given port
    async fn clear_port_events(&mut self, port: LocalPortId) -> Result<PortEventKind, Error<Self::BusError>> {
        if port.0 >= self.port_events.len() as u8 {
            return PdError::InvalidPort.into();
        }

        let event = self.port_events[port.0 as usize];
        self.port_events[port.0 as usize] = PortEventKind::NONE;

        Ok(event)
    }

    /// Returns the current status of the port
    async fn get_port_status(
        &mut self,
        port: LocalPortId,
    ) -> Result<type_c::controller::PortStatus, Error<Self::BusError>> {
        if port.0 >= self.port_status.len() as u8 {
            return PdError::InvalidPort.into();
        }

        Ok(self.port_status[port.0 as usize])
    }

    async fn enable_sink_path(&mut self, port: LocalPortId, enable: bool) -> Result<(), Error<Self::BusError>> {
        debug!("Port{} enable sink path: {}", port.0, enable);
        let mut tps6699x = self.tps6699x.borrow_mut();
        tps6699x.enable_sink_path(port, enable).await
    }

    async fn enable_source(&mut self, port: LocalPortId, enable: bool) -> Result<(), Error<Self::BusError>> {
        debug!("Port{} enable source: {}", port.0, enable);
        let mut tps6699x = self.tps6699x.borrow_mut();
        tps6699x.enable_source(port, enable).await
    }

    async fn set_source_current(
        &mut self,
        port: LocalPortId,
        current: TypecCurrent,
        signal_event: bool,
    ) -> Result<(), Error<Self::BusError>> {
        debug!("Port{} set source current: {}", port.0, current);

        let mut tps6699x = self.tps6699x.borrow_mut();
        let mut port_control = tps6699x.get_port_control(port).await?;
        port_control.set_typec_current(current.into());

        tps6699x.set_port_control(port, port_control).await?;
        if signal_event {
            self.signal_sw_event(port, PortEventKind::NEW_POWER_CONTRACT_AS_PROVIDER);
        }
        Ok(())
    }

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
pub type Tps66994Wrapper<'a, M, B> = ControllerWrapper<TPS66994_NUM_PORTS, Tps6699x<'a, M, B>>;

/// TPS66993 controller wrapper
pub type Tps66993Wrapper<'a, M, B> = ControllerWrapper<TPS66994_NUM_PORTS, Tps6699x<'a, M, B>>;

/// Create a TPS66994 controller wrapper
pub fn tps66994<'a, M: RawMutex, B: I2c>(
    controller: tps6699x::Tps6699x<'a, M, B>,
    controller_id: ControllerId,
    port_ids: [GlobalPortId; TPS66994_NUM_PORTS],
    power_ids: [policy::DeviceId; TPS66994_NUM_PORTS],
) -> Result<ControllerWrapper<TPS66994_NUM_PORTS, Tps6699x<'a, M, B>>, PdError> {
    Ok(ControllerWrapper::new(
        controller::Device::new(controller_id, port_ids.as_slice())?,
        from_fn(|i| policy::device::Device::new(power_ids[i])),
        Tps6699x::new(controller),
    ))
}

/// Create a new TPS66993 controller wrapper
pub fn tps66993<'a, M: RawMutex, B: I2c>(
    controller: tps6699x::Tps6699x<'a, M, B>,
    controller_id: ControllerId,
    port_ids: [GlobalPortId; TPS66993_NUM_PORTS],
    power_ids: [policy::DeviceId; TPS66993_NUM_PORTS],
) -> Result<ControllerWrapper<TPS66993_NUM_PORTS, Tps6699x<'a, M, B>>, PdError> {
    Ok(ControllerWrapper::new(
        controller::Device::new(controller_id, port_ids.as_slice())?,
        from_fn(|i| policy::device::Device::new(power_ids[i])),
        Tps6699x::new(controller),
    ))
}
