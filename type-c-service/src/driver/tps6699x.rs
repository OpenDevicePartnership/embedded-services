use core::array::from_fn;
use core::iter::zip;

use ::tps6699x::registers::field_sets::IntEventBus1;
use ::tps6699x::registers::PlugMode;
use ::tps6699x::{TPS66993_NUM_PORTS, TPS66994_NUM_PORTS};
use embassy_sync::blocking_mutex::raw::RawMutex;
use embedded_hal_async::i2c::I2c;
use embedded_services::power::policy::{self, PowerCapability};
use embedded_services::type_c::controller::{self, Contract, PortStatus, MAX_CONTROLLER_PORTS};
use embedded_services::type_c::event::PortEventKind;
use embedded_services::type_c::{ControllerId, GlobalPortId};
use embedded_services::{debug, info, trace, type_c};
use embedded_usb_pd::pdo::{sink, source, Rdo};
use embedded_usb_pd::type_c::Current as TypecCurrent;
use embedded_usb_pd::{Error, PdError, PortId as LocalPortId};
use tps6699x::asynchronous::embassy as tps6699x;

use crate::wrapper::{Controller, ControllerWrapper};

pub struct Tps6699x<'a, M: RawMutex, B: I2c> {
    port_events: [PortEventKind; MAX_CONTROLLER_PORTS],
    tps6699x: tps6699x::Tps6699x<'a, M, B>,
}

impl<'a, M: RawMutex, B: I2c> Tps6699x<'a, M, B> {
    fn new(tps6699x: tps6699x::Tps6699x<'a, M, B>) -> Self {
        Self {
            port_events: [PortEventKind::NONE; MAX_CONTROLLER_PORTS],
            tps6699x,
        }
    }
}

impl<'a, M: RawMutex, B: I2c> Controller for Tps6699x<'a, M, B> {
    type BusError = B::Error;

    /// Wait for an event on any port
    async fn wait_port_event(&mut self) -> Result<(), Error<Self::BusError>> {
        let interrupts = self.tps6699x.wait_interrupt(false, |_, _| true).await;

        for (i, (interrupt, event)) in zip(interrupts.iter(), self.port_events.iter_mut()).enumerate() {
            //trace!("Interrupt {}: {:#x}", i, interrupt);
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
        let status = self.tps6699x.get_port_status(port).await?;
        trace!("Port{} status: {:#?}", port.0, status);

        let pd_status = self.tps6699x.get_pd_status(port).await?;
        trace!("Port{} PD status: {:#?}", port.0, pd_status);

        let port_control = self.tps6699x.get_port_control(port).await?;
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
            let pdo_raw = self.tps6699x.get_active_pdo_contract(port).await?.active_pdo();
            info!("Raw PDO: {:#X}", pdo_raw);
            let rdo_raw = self.tps6699x.get_active_rdo_contract(port).await?.active_rdo();
            info!("Raw RDO: {:#X}", rdo_raw);

            if pdo_raw != 0 && rdo_raw != 0 {
                // Got a valid explicit contract
                port_status.contract = Some(if pd_status.is_source() {
                    let pdo = source::Pdo::try_from(pdo_raw).map_err(Error::Pd)?;
                    let rdo = Rdo::for_pdo(rdo_raw, pdo);
                    debug!("PDO: {:#?}", pdo);
                    debug!("RDO: {:#?}", rdo);
                    Contract::from(pdo)
                } else {
                    let pdo = sink::Pdo::try_from(pdo_raw).map_err(Error::Pd)?;
                    let rdo = Rdo::for_pdo(rdo_raw, pdo);
                    debug!("PDO: {:#?}", pdo);
                    debug!("RDO: {:#?}", rdo);
                    Contract::from(pdo)
                });
            } else {
                // Determine implicit/default contract
                port_status.contract = Some(if pd_status.is_source() {
                    let current = TypecCurrent::try_from(port_control.typec_current()).map_err(Error::Pd)?;
                    debug!("Port{} type-C source current: {:#?}", port.0, current);
                    Contract::Source(PowerCapability::from(current))
                } else {
                    let current = TypecCurrent::try_from(pd_status.cc_pull_up()).map_err(Error::Pd)?;
                    debug!("Port{} type-C sink current: {:#?}", port.0, current);
                    Contract::Sink(PowerCapability::from(current))
                });
            }
        }

        Ok(port_status)
    }

    async fn enable_sink_path(&mut self, port: LocalPortId, enable: bool) -> Result<(), Error<Self::BusError>> {
        debug!("Port{} enable sink path: {}", port.0, enable);
        self.tps6699x.enable_sink_path(port, enable).await
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
