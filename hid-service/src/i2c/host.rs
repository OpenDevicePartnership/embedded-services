//! I2C<->HID bridge
use core::borrow::{Borrow, BorrowMut};

use embassy_sync::mutex::Mutex;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, with_timeout};
use embedded_services::GlobalRawMutex;
use embedded_services::buffer::OwnedRef;
use embedded_services::comms::{self, Endpoint, EndpointID, External, MailboxDelegate};
use embedded_services::hid::{self, DeviceId, Opcode};
use embedded_services::{error, trace};

use super::{Command as I2cCommand, I2cSlaveAsync};
use crate::Error;

const DEVICE_RESPONSE_TIMEOUT_MS: u64 = 200;
const DATA_READ_TIMEOUT_MS: u64 = 50;

#[derive(Copy, Clone, PartialEq, Eq)]
pub enum Access {
    Read,
    Write,
}

pub struct Host<B: I2cSlaveAsync> {
    id: DeviceId,
    pub tp: Endpoint,
    response: Signal<GlobalRawMutex, Option<hid::Response<'static>>>,
    buffer: OwnedRef<'static, u8>,
    bus: Mutex<GlobalRawMutex, B>,
}

impl<B: I2cSlaveAsync> Host<B> {
    pub fn new(id: DeviceId, bus: B, buffer: OwnedRef<'static, u8>) -> Self {
        Host {
            id,
            tp: Endpoint::uninit(EndpointID::External(External::Host)),
            response: Signal::new(),
            buffer,
            bus: Mutex::new(bus),
        }
    }

    async fn read_bus(&self, timeout_ms: u64, buffer: &mut [u8]) -> Result<(), Error<B::Error>> {
        let mut bus = self.bus.lock().await;
        let result = with_timeout(Duration::from_millis(timeout_ms), bus.respond_to_write(buffer)).await;
        if result.is_err() {
            error!("Response timeout");
            return Err(Error::Hid(hid::Error::Timeout));
        }

        if let Err(e) = result.unwrap() {
            error!("Failed to read from bus");
            return Err(Error::Bus(e));
        }

        Ok(())
    }

    async fn write_bus(&self, timeout_ms: u64, buffer: &[u8]) -> Result<(), Error<B::Error>> {
        let mut bus = self.bus.lock().await;
        // Send response, timeout if the host doesn't read so we don't get stuck here
        trace!("Sending {} bytes", buffer.len());
        let result = with_timeout(Duration::from_millis(timeout_ms), bus.respond_to_read(buffer)).await;
        if result.is_err() {
            error!("Response timeout");
            return Err(Error::Hid(hid::Error::Timeout));
        }

        if let Err(e) = result.unwrap() {
            error!("Failed to rwrite to bus");
            return Err(Error::Bus(e));
        }

        trace!("Response sent");
        Ok(())
    }

    async fn process_command(&self, device: &hid::Device) -> Result<hid::Command<'static>, Error<B::Error>> {
        trace!("Waiting for command");
        let mut cmd = [0u8; 2];
        self.read_bus(DATA_READ_TIMEOUT_MS, &mut cmd).await?;

        let cmd = u16::from_le_bytes(cmd);
        let opcode = Opcode::try_from(cmd);
        if let Err(e) = opcode {
            error!("Invalid command {:#x}", cmd);
            return Err(Error::Hid(e));
        }

        trace!("Command {:#x}", cmd);
        // Get report ID
        let opcode = opcode.unwrap();
        trace!("Opcode {:?}", opcode);
        let report_id = if opcode.requires_report_id() {
            // See if we need to read another byte for the full report ID
            if hid::ReportId::has_extended_report_id(cmd) {
                trace!("Reading extended report ID");
                let mut report_id = [0u8; 1];
                self.read_bus(DATA_READ_TIMEOUT_MS, &mut report_id).await?;

                Some(hid::ReportId(report_id[0]))
            } else {
                Some(hid::ReportId::from_command(cmd))
            }
        } else {
            None
        };

        // Read data from host through data register
        let buffer = if opcode.requires_host_data() || opcode.has_response() {
            let mut addr = [0u8; 2];
            // If the command has a response then we only needed to consume the data register address
            trace!("Waiting for host data access");
            self.read_bus(DATA_READ_TIMEOUT_MS, &mut addr).await?;

            let reg = u16::from_le_bytes(addr);
            if reg != device.regs.data_reg {
                error!("Invalid data register {:#x}", reg);
                return Err(Error::Hid(hid::Error::InvalidRegisterAddress));
            }

            if opcode.requires_host_data() {
                trace!("Waiting for data");
                let mut borrow = self.buffer.borrow_mut();
                let buffer: &mut [u8] = borrow.borrow_mut();

                self.read_bus(DATA_READ_TIMEOUT_MS, &mut buffer[0..2]).await?;

                let length = u16::from_le_bytes([buffer[0], buffer[1]]);
                if buffer.len() < length as usize {
                    error!("Buffer overrun: {}", length);
                    return Err(Error::Hid(hid::Error::InvalidSize(length as usize, buffer.len())));
                }

                trace!("Reading {} bytes", length);
                self.read_bus(DATA_READ_TIMEOUT_MS, &mut buffer[2..length as usize])
                    .await?;
                Some(self.buffer.reference().slice(2..length as usize))
            } else {
                None
            }
        } else {
            None
        };

        // Create command
        let report_type = hid::ReportType::try_from(cmd).ok();
        let command = hid::Command::new(cmd, opcode, report_type, report_id, buffer);
        if let Err(e) = command {
            error!("Invalid command {:?}", e);
            return Err(Error::Hid(hid::Error::InvalidCommand));
        }

        Ok(command.unwrap())
    }

    /// Handle an access to a specific register
    async fn process_register_access(&self) -> Result<(), Error<B::Error>> {
        let mut reg = [0u8; 2];
        trace!("Waiting for register address");
        self.read_bus(DATA_READ_TIMEOUT_MS, &mut reg).await?;

        let reg = u16::from_le_bytes(reg);
        trace!("Register address {:#x}", reg);
        if let Some(device) = hid::get_device(self.id).await {
            let request = if reg == device.regs.hid_desc_reg {
                hid::Request::Descriptor
            } else if reg == device.regs.report_desc_reg {
                hid::Request::ReportDescriptor
            } else if reg == device.regs.input_reg {
                hid::Request::InputReport
            } else if reg == device.regs.command_reg {
                hid::Request::Command(self.process_command(device).await?)
            } else {
                error!("Unexpected request address {:#x}", reg);
                return Err(Error::Hid(hid::Error::InvalidRegisterAddress));
            };

            hid::send_request(&self.tp, self.id, request)
                .await
                .map_err(|_| Error::Hid(hid::Error::Transport))?;

            trace!("Request processed");
            Ok(())
        } else {
            error!("Invalid device id {}", self.id.0);
            Err(Error::Hid(hid::Error::InvalidDevice))
        }
    }

    async fn process_read(&self) -> Result<(), Error<B::Error>> {
        trace!("Got input report read request");
        hid::send_request(&self.tp, self.id, hid::Request::InputReport)
            .await
            .map_err(|_| Error::Hid(hid::Error::Transport))
    }

    /// Process a request from the host
    pub async fn wait_request(&self) -> Result<Access, Error<B::Error>> {
        // Wait for HID register address
        let mut bus = self.bus.lock().await;
        loop {
            trace!("Waiting for host");
            match bus.listen().await {
                Err(e) => {
                    error!("Bus error");
                    return Err(Error::Bus(e));
                }
                Ok(cmd) => match cmd {
                    I2cCommand::Probe => continue,
                    I2cCommand::Read => return Ok(Access::Read),
                    I2cCommand::Write => return Ok(Access::Write),
                },
            }
        }
    }

    pub async fn process_request(&self, access: Access) -> Result<(), Error<B::Error>> {
        match access {
            Access::Read => self.process_read().await,
            Access::Write => self.process_register_access().await,
        }
    }

    pub async fn send_response(&self) -> Result<(), Error<B::Error>> {
        if let Some(response) = self.response.wait().await {
            match response {
                hid::Response::Descriptor(_) => trace!("Sending descriptor"),
                hid::Response::ReportDescriptor(_) => trace!("Sending report descriptor"),
                hid::Response::InputReport(_) => trace!("Sending input report"),
                hid::Response::FeatureReport(_) => trace!("Sending feature report"),
                hid::Response::Command(_) => trace!("Sending command"),
            }

            // Wait for the read from the host
            // Input reports just a read so we don't need to wait for one
            if !matches!(response, hid::Response::InputReport(_)) {
                let mut bus = self.bus.lock().await;
                match bus.listen().await {
                    Err(e) => {
                        error!("Bus error");
                        return Err(Error::Bus(e));
                    }
                    Ok(cmd) => {
                        if cmd != I2cCommand::Read {
                            error!("Expected read, got {:?}", cmd);
                            return Err(Error::Hid(hid::Error::Timeout));
                        }
                    }
                }
            }

            match response {
                hid::Response::Descriptor(data)
                | hid::Response::ReportDescriptor(data)
                | hid::Response::InputReport(data)
                | hid::Response::FeatureReport(data) => {
                    let bytes = data.borrow();
                    self.write_bus(DEVICE_RESPONSE_TIMEOUT_MS, bytes.borrow()).await
                }
                hid::Response::Command(cmd) => match cmd {
                    hid::CommandResponse::GetIdle(freq) => {
                        let freq: u16 = freq.into();
                        let mut buffer = [0u8; 2];
                        buffer.copy_from_slice(freq.to_le_bytes().as_slice());
                        self.write_bus(DEVICE_RESPONSE_TIMEOUT_MS, &buffer).await
                    }
                    hid::CommandResponse::GetProtocol(protocol) => {
                        let protocol: u16 = protocol.into();
                        let mut buffer = [0u8; 2];
                        buffer.copy_from_slice(protocol.to_le_bytes().as_slice());
                        self.write_bus(DEVICE_RESPONSE_TIMEOUT_MS, &buffer).await
                    }
                    hid::CommandResponse::Vendor => Ok(()),
                },
            }
        } else {
            Ok(())
        }
    }

    pub async fn process(&self) -> Result<(), Error<B::Error>> {
        let access = self.wait_request().await?;
        self.process_request(access).await?;
        self.send_response().await
    }
}

impl<B: I2cSlaveAsync> MailboxDelegate for Host<B> {
    fn receive(&self, message: &comms::Message) -> Result<(), comms::MailboxDelegateError> {
        let hid_msg = message
            .data
            .get::<hid::Message>()
            .ok_or(comms::MailboxDelegateError::MessageNotFound)?;

        match hid_msg.data {
            hid::MessageData::Response(ref response) => {
                self.response.signal(response.clone());
                Ok(())
            }
            _ if message.to != EndpointID::External(External::Host) => {
                Err(comms::MailboxDelegateError::InvalidDestination)
            }
            _ if hid_msg.id != self.id => Err(comms::MailboxDelegateError::InvalidData),
            _ => Err(comms::MailboxDelegateError::Other),
        }
    }
}
