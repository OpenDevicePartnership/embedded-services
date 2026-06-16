use core::borrow::BorrowMut;

use embassy_sync::mutex::Mutex;
use embassy_time::{with_timeout, Duration};
use embedded_hal_async::i2c::{AddressMode, I2c};
use embedded_services::hid::{DeviceContainer, InvalidSizeError, Opcode, Response};
use embedded_services::{buffer::*, GlobalRawMutex};
use embedded_services::{error, hid, info, trace};

use crate::Error;

const LENGTH_PREFIX_SIZE: usize = 2;

/// Timeout configuration for I2C HID device operations.
pub struct Config {
    /// Timeout for descriptor reads and commands.
    pub device_response_timeout: Duration,
    /// Timeout for input reports and feature data reads.
    pub data_read_timeout: Duration,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            device_response_timeout: Duration::from_millis(200),
            data_read_timeout: Duration::from_millis(50),
        }
    }
}

pub struct Device<A: AddressMode + Copy, B: I2c<A>> {
    device: hid::Device,
    buffer: OwnedRef<'static, u8>,
    address: A,
    descriptor: Mutex<GlobalRawMutex, Option<hid::Descriptor>>,
    bus: Mutex<GlobalRawMutex, B>,
    timeout_config: Config,
}

impl<A: AddressMode + Copy, B: I2c<A>> Device<A, B> {
    pub fn new(
        id: hid::DeviceId,
        address: A,
        bus: B,
        regs: hid::RegisterFile,
        buffer: OwnedRef<'static, u8>,
        timeout_config: Config,
    ) -> Self {
        Self {
            device: hid::Device::new(id, regs),
            buffer,
            address,
            descriptor: Mutex::new(None),
            bus: Mutex::new(bus),
            timeout_config,
        }
    }

    async fn get_hid_descriptor(&self) -> Result<hid::Descriptor, Error<B::Error>> {
        {
            let descriptor = self.descriptor.lock().await;
            if descriptor.is_some() {
                return Ok(descriptor.unwrap());
            }
        }
        let mut bus = self.bus.lock().await;
        let mut borrow = self.buffer.borrow_mut().map_err(Error::Buffer)?;
        let mut reg = [0u8; 2];
        let buf: &mut [u8] = borrow.borrow_mut();
        let buf_len = buf.len();
        let buf = buf
            .get_mut(0..hid::DESCRIPTOR_LEN)
            .ok_or(Error::Hid(hid::Error::InvalidSize(InvalidSizeError {
                expected: hid::DESCRIPTOR_LEN,
                actual: buf_len,
            })))?;

        reg.copy_from_slice(&self.device.regs.hid_desc_reg.to_le_bytes());
        with_timeout(
            self.timeout_config.device_response_timeout,
            bus.write_read(self.address, &reg, buf),
        )
        .await
        .map_err(|_| {
            error!("Read HID descriptor timeout");
            Error::Hid(hid::Error::Timeout)
        })?
        .map_err(|e| {
            error!("Failed to read HID descriptor");
            Error::Bus(e)
        })?;

        let res = hid::Descriptor::decode_from_slice(buf);
        match res {
            Ok(desc) => {
                info!("HID descriptor: {:#?}", desc);
                let mut descriptor = self.descriptor.lock().await;
                *descriptor = Some(desc);
                Ok(desc)
            }
            Err(e) => {
                error!("Failed to deserialize HID descriptor: {:?}", e);
                Err(Error::Hid(hid::Error::Serialize))
            }
        }
    }

    pub async fn read_hid_descriptor(&self) -> Result<SharedRef<'static, u8>, Error<B::Error>> {
        let desc = self.get_hid_descriptor().await?;

        let mut borrow = self.buffer.borrow_mut().map_err(Error::Buffer)?;
        let buf: &mut [u8] = borrow.borrow_mut();

        let len = desc.encode_into_slice(buf).map_err(Error::Hid)?;
        trace!("HID descriptor length: {}", len);
        self.buffer.reference().slice(0..len).map_err(Error::Buffer)
    }

    pub async fn read_report_descriptor(&self) -> Result<SharedRef<'static, u8>, Error<B::Error>> {
        info!("Sending report descriptor");
        let desc = self.get_hid_descriptor().await?;

        let mut borrow = self.buffer.borrow_mut().map_err(Error::Buffer)?;
        let buf: &mut [u8] = borrow.borrow_mut();
        let buffer_len = buf.len();
        let reg = desc.w_report_desc_register.to_le_bytes();
        let len = desc.w_report_desc_length as usize;

        let mut bus = self.bus.lock().await;
        with_timeout(
            self.timeout_config.device_response_timeout,
            bus.write_read(
                self.address,
                &reg,
                buf.get_mut(0..len)
                    .ok_or(Error::Hid(hid::Error::InvalidSize(InvalidSizeError {
                        expected: len,
                        actual: buffer_len,
                    })))?,
            ),
        )
        .await
        .map_err(|_| {
            error!("Read report descriptor timeout");
            Error::Hid(hid::Error::Timeout)
        })?
        .map_err(|e| {
            error!("Failed to read report descriptor");
            Error::Bus(e)
        })?;

        self.buffer.reference().slice(0..len).map_err(Error::Buffer)
    }

    pub async fn handle_input_report(&self) -> Result<SharedRef<'static, u8>, Error<B::Error>> {
        info!("Handling input report");
        let desc = self.get_hid_descriptor().await?;

        let mut borrow = self.buffer.borrow_mut().map_err(Error::Buffer)?;
        let buf: &mut [u8] = borrow.borrow_mut();
        let buffer_len = buf.len();
        let buf = buf
            .get_mut(0..desc.w_max_input_length as usize)
            .ok_or(Error::Hid(hid::Error::InvalidSize(InvalidSizeError {
                expected: desc.w_max_input_length as usize,
                actual: buffer_len,
            })))?;

        let mut bus = self.bus.lock().await;
        with_timeout(self.timeout_config.data_read_timeout, bus.read(self.address, buf))
            .await
            .map_err(|_| {
                error!("Read input report timeout");
                Error::Hid(hid::Error::Timeout)
            })?
            .map_err(|e| {
                error!("Failed to read input report");
                Error::Bus(e)
            })?;

        self.buffer
            .reference()
            .slice(0..desc.w_max_input_length as usize)
            .map_err(Error::Buffer)
    }

    pub async fn handle_command(
        &self,
        cmd: &hid::Command<'static>,
    ) -> Result<Option<Response<'static>>, Error<B::Error>> {
        info!("Handling command");

        let desc = self.get_hid_descriptor().await?;
        let (command_reg, data_reg) = (desc.w_command_register, desc.w_data_register);

        let mut borrow = self.buffer.borrow_mut().map_err(Error::Buffer)?;
        let buf: &mut [u8] = borrow.borrow_mut();
        let buffer_len = buf.len();

        let opcode: Opcode = cmd.into();

        if opcode.has_response() {
            // Commands that require a response (GetReport, GetIdle, GetProtocol)
            // have an upper limit of 7 bytes for the command
            let mut temp_w_buf = [0u8; 7];

            let len = cmd
                .encode_into_slice(&mut temp_w_buf, Some(command_reg), Some(data_reg))
                .map_err(|_| {
                    error!("Failed to serialize command");
                    Error::Hid(hid::Error::Serialize)
                })?;

            let (response_size, constrained) = match cmd {
                hid::Command::GetReport {
                    expected_payload_size: Some(expected_payload_size),
                    ..
                } => (*expected_payload_size as usize + LENGTH_PREFIX_SIZE, true),
                _ => (buffer_len, false),
            };
            let read_buf =
                buf.get_mut(0..response_size)
                    .ok_or(Error::Hid(hid::Error::InvalidSize(InvalidSizeError {
                        expected: response_size,
                        actual: buffer_len,
                    })))?;

            let mut bus = self.bus.lock().await;

            with_timeout(
                self.timeout_config.device_response_timeout,
                bus.write_read(
                    self.address,
                    temp_w_buf
                        .get(..len)
                        .ok_or(Error::Hid(hid::Error::InvalidSize(InvalidSizeError {
                            expected: len,
                            actual: temp_w_buf.len(),
                        })))?,
                    read_buf,
                ),
            )
            .await
            .map_err(|_| {
                error!("Command write_read timeout");
                Error::Hid(hid::Error::Timeout)
            })?
            .map_err(|e| {
                error!("Failed to execute command write_read");
                Error::Bus(e)
            })?;

            if constrained {
                let actual_frame_len = read_buf
                    .first_chunk::<LENGTH_PREFIX_SIZE>()
                    .map(|b| u16::from_le_bytes(*b) as usize)
                    .ok_or(Error::Hid(hid::Error::InvalidSize(InvalidSizeError {
                        expected: LENGTH_PREFIX_SIZE,
                        actual: read_buf.len(),
                    })))?;
                if actual_frame_len > response_size {
                    error!(
                        "Length mismatch: declared={} expected={}",
                        actual_frame_len, response_size
                    );
                    return Err(Error::Hid(hid::Error::InvalidSize(InvalidSizeError {
                        expected: response_size,
                        actual: actual_frame_len,
                    })));
                }
            }

            Ok(Some(Response::FeatureReport(
                self.buffer.reference().slice(0..response_size).map_err(Error::Buffer)?,
            )))
        } else {
            let len = cmd
                .encode_into_slice(
                    buf,
                    Some(command_reg),
                    if opcode.requires_host_data() {
                        Some(data_reg)
                    } else {
                        None
                    },
                )
                .map_err(|_| {
                    error!("Failed to serialize command");
                    Error::Hid(hid::Error::Serialize)
                })?;

            let mut bus = self.bus.lock().await;
            with_timeout(
                self.timeout_config.device_response_timeout,
                bus.write(
                    self.address,
                    buf.get(..len)
                        .ok_or(Error::Hid(hid::Error::InvalidSize(InvalidSizeError {
                            expected: len,
                            actual: buffer_len,
                        })))?,
                ),
            )
            .await
            .map_err(|_| {
                error!("Write command timeout");
                Error::Hid(hid::Error::Timeout)
            })?
            .map_err(|e| {
                error!("Failed to write command");
                Error::Bus(e)
            })?;

            Ok(None)
        }
    }

    pub async fn process_request(&self) -> Result<(), Error<B::Error>> {
        let req = self.device.wait_request().await;

        let response = match req {
            hid::Request::Descriptor => {
                let desc = self.read_hid_descriptor().await?;
                Some(hid::Response::Descriptor(desc))
            }
            hid::Request::ReportDescriptor => {
                let desc = self.read_report_descriptor().await?;
                Some(hid::Response::ReportDescriptor(desc))
            }
            hid::Request::InputReport => {
                let report = self.handle_input_report().await?;
                Some(hid::Response::InputReport(report))
            }
            hid::Request::Command(cmd) => self.handle_command(&cmd).await?,
            _ => {
                error!("Unimplemented HID request");
                None
            }
        };

        self.device
            .send_response(response)
            .await
            .map_err(|_| Error::Hid(hid::Error::Transport))
    }
}

impl<A: AddressMode + Copy, B: I2c<A>> DeviceContainer for Device<A, B> {
    fn get_hid_device(&self) -> &hid::Device {
        &self.device
    }
}
