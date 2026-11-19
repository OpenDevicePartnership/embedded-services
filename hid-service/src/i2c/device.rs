use core::borrow::BorrowMut;

use embassy_sync::mutex::Mutex;
use embassy_time::{Duration, with_timeout};
use embedded_hal_async::i2c::{AddressMode, I2c};
use embedded_services::hid::{DeviceContainer, InvalidSizeError, Opcode, Response};
use embedded_services::{GlobalRawMutex, SyncCell, buffer::*};
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

/// I2C HID device with metadata cached for input reports and commands.
///
/// Operations on a device must be serialized, as in the passthrough request loop:
/// its response buffer and descriptor cache are shared between operations.
pub struct Device<A: AddressMode + Copy, B: I2c<A>> {
    device: hid::Device,
    buffer: OwnedRef<'static, u8>,
    address: A,
    descriptor: SyncCell<Option<hid::Descriptor>>,
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
            descriptor: SyncCell::new(None),
            bus: Mutex::new(bus),
            timeout_config,
        }
    }

    async fn refresh_hid_descriptor(&self) -> Result<hid::Descriptor, Error<B::Error>> {
        // A refresh may follow a mode change. Do not retain potentially stale
        // metadata if the read fails or its future is cancelled.
        self.descriptor.set(None);
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
                self.descriptor.set(Some(desc));
                Ok(desc)
            }
            Err(e) => {
                error!("Failed to deserialize HID descriptor: {:?}", e);
                Err(Error::Hid(hid::Error::Serialize))
            }
        }
    }

    async fn get_or_fetch_hid_descriptor(&self) -> Result<hid::Descriptor, Error<B::Error>> {
        if let Some(descriptor) = self.descriptor.get() {
            return Ok(descriptor);
        }
        self.refresh_hid_descriptor().await
    }

    /// Fetch current HID metadata and cache it for input reports and commands.
    ///
    /// With serialized operations, a failed or cancelled metadata refresh leaves
    /// the cache empty. Mode changes outside descriptor requests are not detected
    /// automatically.
    pub async fn read_hid_descriptor(&self) -> Result<SharedRef<'static, u8>, Error<B::Error>> {
        let desc = self.refresh_hid_descriptor().await?;

        let mut borrow = self.buffer.borrow_mut().map_err(Error::Buffer)?;
        let buf: &mut [u8] = borrow.borrow_mut();

        let len = desc.encode_into_slice(buf).map_err(Error::Hid)?;
        trace!("HID descriptor length: {}", len);
        self.buffer.reference().slice(0..len).map_err(Error::Buffer)
    }

    /// Refresh HID metadata before using its report-descriptor register and length.
    ///
    /// With serialized operations, successfully refreshed metadata remains cached
    /// even if reading the report descriptor subsequently fails or is cancelled.
    pub async fn read_report_descriptor(&self) -> Result<SharedRef<'static, u8>, Error<B::Error>> {
        info!("Sending report descriptor");
        let desc = self.refresh_hid_descriptor().await?;

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
        let desc = self.get_or_fetch_hid_descriptor().await?;

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

        let desc = self.get_or_fetch_hid_descriptor().await?;
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

            let returned_len = if constrained {
                let actual_frame_len = read_buf
                    .first_chunk::<LENGTH_PREFIX_SIZE>()
                    .map(|b| u16::from_le_bytes(*b) as usize)
                    .ok_or(Error::Hid(hid::Error::InvalidSize(InvalidSizeError {
                        expected: LENGTH_PREFIX_SIZE,
                        actual: read_buf.len(),
                    })))?;
                if actual_frame_len < LENGTH_PREFIX_SIZE || actual_frame_len > response_size {
                    error!(
                        "Length mismatch: declared={} expected<={} min={}",
                        actual_frame_len, response_size, LENGTH_PREFIX_SIZE
                    );
                    return Err(Error::Hid(hid::Error::InvalidSize(InvalidSizeError {
                        expected: response_size,
                        actual: actual_frame_len,
                    })));
                }
                actual_frame_len
            } else {
                response_size
            };

            Ok(Some(Response::FeatureReport(
                self.buffer.reference().slice(0..returned_len).map_err(Error::Buffer)?,
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

#[cfg(test)]
mod tests {
    //! Contract tests using the real device implementation and a scripted physical bus.

    #![allow(clippy::expect_used, clippy::indexing_slicing, clippy::panic, clippy::unwrap_used)]

    extern crate std;

    use core::borrow::Borrow;
    use core::future::{Future, pending};
    use core::pin::pin;
    use core::task::{Context, Poll, Waker};

    use embassy_futures::{block_on, join::join};
    use embedded_hal::i2c::{ErrorKind, ErrorType};
    use embedded_hal_async::i2c::Operation;
    use embedded_services::comms;
    use std::{boxed::Box, cell::RefCell, collections::VecDeque, rc::Rc, vec, vec::Vec};

    use super::*;
    use crate::i2c::{Command as SlaveCommand, Host, HostConfig, I2cSlaveAsync};

    // Synthetic modes, not captured ELAN bootloader/application descriptors.
    // Report register/length, input length, command/data registers, and version
    // differ to exercise the generic cache policy independently of hardware.
    const BOOT_DESCRIPTOR: [u8; 30] = [
        30, 0, 0, 1, 4, 0, 0x22, 0x11, 3, 0, 8, 0, 4, 0, 8, 0, 5, 0, 6, 0, 0x34, 0x12, 0x78, 0x56, 1, 0, 0, 0, 0, 0,
    ];
    const APP_DESCRIPTOR: [u8; 30] = [
        30, 0, 0, 1, 6, 0, 0x44, 0x33, 3, 0, 10, 0, 4, 0, 8, 0, 0x34, 0x12, 0x78, 0x56, 0x34, 0x12, 0x78, 0x56, 2, 0,
        0, 0, 0, 0,
    ];

    // ELAN HC4909-20H5V0 I2C Programming Guide v1.0, §2.1. Literal wire bytes,
    // independent of the production encoder. Use numeric wMaxInputLength 0x002C,
    // consistent with the bytes 0..43 layout in §4.2, not its conflicting prose.
    const ELAN_DESCRIPTOR: [u8; 30] = [
        0x1e, 0x00, 0x00, 0x01, 0x69, 0x04, 0x02, 0x00, 0x03, 0x00, 0x2c, 0x00, 0x04, 0x00, 0x0a, 0x00, 0x05, 0x00,
        0x06, 0x00, 0xf3, 0x04, 0x8f, 0x33, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00,
    ];

    enum Transfer {
        WriteRead { write: Vec<u8>, reply: Reply },
        Read(Vec<u8>),
        Write(Vec<u8>),
    }

    #[derive(Clone)]
    enum Reply {
        Data(Vec<u8>),
        Error(Vec<u8>),
        Pending,
        PartialPending(Vec<u8>),
    }

    fn write_read(write: &[u8], read: &[u8]) -> Transfer {
        Transfer::WriteRead {
            write: write.to_vec(),
            reply: Reply::Data(read.to_vec()),
        }
    }

    fn descriptor(bytes: [u8; 30]) -> Transfer {
        write_read(&[1, 0], &bytes)
    }

    #[derive(Clone)]
    struct Bus {
        address: u8,
        transfers: Rc<RefCell<VecDeque<Transfer>>>,
    }

    impl Bus {
        fn new(transfers: impl IntoIterator<Item = Transfer>) -> Self {
            Self::at_address(0x2c, transfers)
        }

        fn at_address(address: u8, transfers: impl IntoIterator<Item = Transfer>) -> Self {
            Self {
                address,
                transfers: Rc::new(RefCell::new(transfers.into_iter().collect())),
            }
        }

        fn assert_finished(&self) {
            assert!(
                self.transfers.as_ref().borrow().is_empty(),
                "expected I2C transactions were skipped"
            );
        }
    }

    impl ErrorType for Bus {
        type Error = ErrorKind;
    }

    impl I2c for Bus {
        async fn transaction(&mut self, address: u8, operations: &mut [Operation<'_>]) -> Result<(), Self::Error> {
            assert_eq!(address, self.address);
            let transfer = self
                .transfers
                .as_ref()
                .borrow_mut()
                .pop_front()
                .expect("unexpected I2C transaction");
            match (transfer, operations) {
                (
                    Transfer::WriteRead { write, reply },
                    [Operation::Write(actual_write), Operation::Read(actual_read)],
                ) => {
                    assert_eq!(*actual_write, write);
                    match reply {
                        Reply::Data(read) => {
                            assert_eq!(actual_read.len(), read.len());
                            actual_read.copy_from_slice(&read);
                        }
                        Reply::Error(prefix) => {
                            actual_read[..prefix.len()].copy_from_slice(&prefix);
                            return Err(ErrorKind::Other);
                        }
                        Reply::Pending => return pending().await,
                        Reply::PartialPending(prefix) => {
                            actual_read[..prefix.len()].copy_from_slice(&prefix);
                            return pending().await;
                        }
                    }
                }
                (Transfer::Read(read), [Operation::Read(actual)]) => actual.copy_from_slice(&read),
                (Transfer::Write(write), [Operation::Write(actual)]) => assert_eq!(*actual, write),
                _ => panic!("unexpected I2C transaction shape"),
            }
            Ok(())
        }
    }

    fn device(bus: Bus) -> Device<u8, Bus> {
        device_with_capacity(bus, 64)
    }

    fn buffer(capacity: usize) -> OwnedRef<'static, u8> {
        let bytes = Box::leak(vec![0; capacity].into_boxed_slice());
        // SAFETY: The leaked storage has no other users. Exactly one OwnedRef is
        // created; subsequent access uses the production buffer's borrow guards.
        let buffer = Box::leak(Box::new(unsafe { Buffer::new(bytes) }));
        unsafe { buffer.as_owned() }
    }

    fn device_with_capacity(bus: Bus, capacity: usize) -> Device<u8, Bus> {
        let address = bus.address;
        Device::new(
            hid::DeviceId(0),
            address,
            bus,
            hid::RegisterFile::default(),
            buffer(capacity),
            Config {
                device_response_timeout: Duration::from_millis(10),
                ..Config::default()
            },
        )
    }

    fn bytes(response: SharedRef<'static, u8>) -> Vec<u8> {
        let borrow = response.borrow().unwrap();
        let bytes: &[u8] = borrow.borrow();
        bytes.to_vec()
    }

    #[test]
    fn repeated_hid_descriptor_reads_return_current_metadata() {
        let bus = Bus::new([descriptor(BOOT_DESCRIPTOR), descriptor(APP_DESCRIPTOR)]);
        let device = device(bus.clone());

        assert_eq!(bytes(block_on(device.read_hid_descriptor()).unwrap()), BOOT_DESCRIPTOR);
        assert_eq!(bytes(block_on(device.read_hid_descriptor()).unwrap()), APP_DESCRIPTOR);
        bus.assert_finished();
    }

    // Both public descriptor operations are refresh boundaries. Test them through
    // their public behavior, rather than inspecting the cache's representation.
    #[derive(Clone, Copy)]
    enum Refresh {
        Hid,
        Report,
    }

    impl Refresh {
        async fn read(self, device: &Device<u8, Bus>) -> Result<SharedRef<'static, u8>, Error<ErrorKind>> {
            match self {
                Self::Hid => device.read_hid_descriptor().await,
                Self::Report => device.read_report_descriptor().await,
            }
        }
    }

    #[test]
    fn failed_refresh_prevents_reuse_of_old_metadata() {
        let mut malformed = APP_DESCRIPTOR;
        malformed[26] = 1; // Nonzero reserved byte must not become cached metadata.

        for (reply, bus_error) in [
            (Reply::Error(vec![]), true),
            (Reply::Error(APP_DESCRIPTOR[..25].to_vec()), true),
            (Reply::Data(malformed.to_vec()), false),
        ] {
            for refresh in [Refresh::Hid, Refresh::Report] {
                let input = vec![10, 0, 1, 2, 3, 4, 5, 6, 7, 8];
                let bus = Bus::new([
                    descriptor(BOOT_DESCRIPTOR),
                    Transfer::WriteRead {
                        write: vec![1, 0],
                        reply: reply.clone(),
                    },
                    descriptor(APP_DESCRIPTOR),
                    Transfer::Read(input.clone()),
                ]);
                let device = device(bus.clone());
                bytes(block_on(device.read_hid_descriptor()).unwrap());

                let result = block_on(refresh.read(&device));
                if bus_error {
                    assert!(matches!(result, Err(Error::Bus(ErrorKind::Other))));
                } else {
                    assert!(matches!(result, Err(Error::Hid(hid::Error::Serialize))));
                }
                assert_eq!(bytes(block_on(device.handle_input_report()).unwrap()), input);
                bus.assert_finished();
            }
        }
    }

    #[test]
    fn cancelled_refresh_prevents_reuse_of_old_metadata() {
        for refresh in [Refresh::Hid, Refresh::Report] {
            let input = vec![10, 0, 1, 2, 3, 4, 5, 6, 7, 8];
            let bus = Bus::new([
                descriptor(BOOT_DESCRIPTOR),
                Transfer::WriteRead {
                    write: vec![1, 0],
                    reply: Reply::Pending,
                },
                descriptor(APP_DESCRIPTOR),
                Transfer::Read(input.clone()),
            ]);
            let device = device(bus.clone());
            bytes(block_on(device.read_hid_descriptor()).unwrap());

            {
                let mut future = pin!(refresh.read(&device));
                let mut context = Context::from_waker(Waker::noop());
                assert!(matches!(future.as_mut().poll(&mut context), Poll::Pending));
            }

            assert_eq!(bytes(block_on(device.handle_input_report()).unwrap()), input);
            bus.assert_finished();
        }
    }

    #[test]
    fn timed_out_refresh_prevents_reuse_of_old_metadata() {
        for refresh in [Refresh::Hid, Refresh::Report] {
            let input = vec![10, 0, 1, 2, 3, 4, 5, 6, 7, 8];
            let bus = Bus::new([
                descriptor(BOOT_DESCRIPTOR),
                Transfer::WriteRead {
                    write: vec![1, 0],
                    reply: Reply::Pending,
                },
                descriptor(APP_DESCRIPTOR),
                Transfer::Read(input.clone()),
            ]);
            let device = device(bus.clone());
            bytes(block_on(device.read_hid_descriptor()).unwrap());

            assert!(matches!(
                block_on(refresh.read(&device)),
                Err(Error::Hid(hid::Error::Timeout))
            ));
            assert_eq!(bytes(block_on(device.handle_input_report()).unwrap()), input);
            bus.assert_finished();
        }
    }

    #[test]
    fn repeated_report_reads_use_current_register_and_length() {
        let bus = Bus::new([
            descriptor(BOOT_DESCRIPTOR),
            write_read(&[0x22, 0x11], &[5, 1, 9, 2]),
            descriptor(APP_DESCRIPTOR),
            write_read(&[0x44, 0x33], &[5, 13, 9, 5, 0xa1, 1]),
        ]);
        let device = device(bus.clone());

        assert_eq!(bytes(block_on(device.read_report_descriptor()).unwrap()), [5, 1, 9, 2]);
        assert_eq!(
            bytes(block_on(device.read_report_descriptor()).unwrap()),
            [5, 13, 9, 5, 0xa1, 1]
        );
        bus.assert_finished();
    }

    #[test]
    fn refreshed_metadata_is_reused_by_input_and_commands_without_descriptor_io() {
        for refresh in [Refresh::Hid, Refresh::Report] {
            let input = vec![10, 0, 1, 2, 3, 4, 5, 6, 7, 8];
            let mut transfers = vec![descriptor(BOOT_DESCRIPTOR), descriptor(APP_DESCRIPTOR)];
            if matches!(refresh, Refresh::Report) {
                transfers.push(write_read(&[0x44, 0x33], &[5, 13, 9, 5, 0xa1, 1]));
            }
            transfers.extend([
                Transfer::Read(input.clone()),
                write_read(&[0x34, 0x12, 0x31, 2, 0x78, 0x56], &[4, 0, 0xab, 0xcd]),
                Transfer::Write(vec![0x34, 0x12, 0, 8]),
            ]);
            let bus = Bus::new(transfers);
            let device = device(bus.clone());
            bytes(block_on(device.read_hid_descriptor()).unwrap());
            let response = bytes(block_on(refresh.read(&device)).unwrap());
            match refresh {
                Refresh::Hid => assert_eq!(response, APP_DESCRIPTOR),
                Refresh::Report => assert_eq!(response, [5, 13, 9, 5, 0xa1, 1]),
            }

            assert_eq!(bytes(block_on(device.handle_input_report()).unwrap()), input);
            let command = hid::Command::GetReport {
                report_type: hid::ReportType::Feature,
                report_id: hid::ReportId(1),
                expected_payload_size: Some(2),
            };
            let Some(Response::FeatureReport(response)) = block_on(device.handle_command(&command)).unwrap() else {
                panic!("expected a feature report");
            };
            assert_eq!(bytes(response), [4, 0, 0xab, 0xcd]);
            assert!(
                block_on(device.handle_command(&hid::Command::SetPower(hid::PowerState::On)))
                    .unwrap()
                    .is_none()
            );
            bus.assert_finished();
        }
    }

    #[test]
    fn cold_input_read_fetches_metadata_once() {
        let input = vec![8, 0, 1, 2, 3, 4, 5, 6];
        let bus = Bus::new([
            descriptor(BOOT_DESCRIPTOR),
            Transfer::Read(input.clone()),
            Transfer::Read(input.clone()),
        ]);
        let device = device(bus.clone());

        assert_eq!(bytes(block_on(device.handle_input_report()).unwrap()), input);
        assert_eq!(bytes(block_on(device.handle_input_report()).unwrap()), input);
        bus.assert_finished();
    }

    #[test]
    fn cold_command_fetches_metadata_once() {
        let bus = Bus::new([
            descriptor(BOOT_DESCRIPTOR),
            Transfer::Write(vec![5, 0, 0, 8]),
            Transfer::Write(vec![5, 0, 0, 8]),
        ]);
        let device = device(bus.clone());
        let command = hid::Command::SetPower(hid::PowerState::On);

        assert!(block_on(device.handle_command(&command)).unwrap().is_none());
        assert!(block_on(device.handle_command(&command)).unwrap().is_none());
        bus.assert_finished();
    }

    #[test]
    fn failed_buffer_borrow_during_refresh_invalidates_old_metadata() {
        for refresh in [Refresh::Hid, Refresh::Report] {
            let input = vec![10, 0, 1, 2, 3, 4, 5, 6, 7, 8];
            let bus = Bus::new([
                descriptor(BOOT_DESCRIPTOR),
                descriptor(APP_DESCRIPTOR),
                Transfer::Read(input.clone()),
            ]);
            let device = device(bus.clone());
            let response = block_on(device.read_hid_descriptor()).unwrap();
            let borrow = response.borrow().unwrap();

            assert!(matches!(
                block_on(refresh.read(&device)),
                Err(Error::Buffer(embedded_services::buffer::Error::BorrowedImmutably))
            ));
            drop(borrow);

            assert_eq!(bytes(block_on(device.handle_input_report()).unwrap()), input);
            bus.assert_finished();
        }
    }

    #[test]
    fn failed_report_transfer_keeps_successfully_refreshed_metadata() {
        let input = vec![10, 0, 1, 2, 3, 4, 5, 6, 7, 8];
        let bus = Bus::new([
            descriptor(BOOT_DESCRIPTOR),
            descriptor(APP_DESCRIPTOR),
            Transfer::WriteRead {
                write: vec![0x44, 0x33],
                reply: Reply::Error(vec![]),
            },
            Transfer::Read(input.clone()),
        ]);
        let device = device(bus.clone());
        bytes(block_on(device.read_hid_descriptor()).unwrap());

        assert!(matches!(block_on(device.read_report_descriptor()), Err(Error::Bus(_))));
        assert_eq!(bytes(block_on(device.handle_input_report()).unwrap()), input);
        bus.assert_finished();
    }

    #[test]
    fn cancelled_report_transfer_keeps_successfully_refreshed_metadata() {
        let input = vec![10, 0, 1, 2, 3, 4, 5, 6, 7, 8];
        let bus = Bus::new([
            descriptor(BOOT_DESCRIPTOR),
            descriptor(APP_DESCRIPTOR),
            Transfer::WriteRead {
                write: vec![0x44, 0x33],
                reply: Reply::Pending,
            },
            Transfer::Read(input.clone()),
        ]);
        let device = device(bus.clone());
        bytes(block_on(device.read_hid_descriptor()).unwrap());

        {
            let mut future = pin!(device.read_report_descriptor());
            let mut context = Context::from_waker(Waker::noop());
            assert!(matches!(future.as_mut().poll(&mut context), Poll::Pending));
        }

        assert_eq!(bytes(block_on(device.handle_input_report()).unwrap()), input);
        bus.assert_finished();
    }

    #[test]
    fn documented_touchpad_report_is_returned_without_truncation() {
        // Guide §§2.1, 3.2 and 3.3: 30-byte HID metadata, report register 2,
        // report length 0x0469. The opaque report payload is synthetic; this
        // checks transport capacity, not the contents of ELAN's report grammar.
        let report: Vec<u8> = (0..1129).map(|offset| (offset % 251) as u8).collect();
        let mut input = vec![0; 44];
        input[..3].copy_from_slice(&[44, 0, 0x52]);
        let bus = Bus::at_address(
            0x15,
            [
                descriptor(ELAN_DESCRIPTOR),
                descriptor(ELAN_DESCRIPTOR),
                write_read(&[2, 0], &report),
                Transfer::Read(input.clone()),
            ],
        );
        let device = device_with_capacity(bus.clone(), 1129);

        assert_eq!(bytes(block_on(device.read_hid_descriptor()).unwrap()), ELAN_DESCRIPTOR);
        assert_eq!(bytes(block_on(device.read_report_descriptor()).unwrap()), report);
        assert_eq!(bytes(block_on(device.handle_input_report()).unwrap()), input);
        bus.assert_finished();
    }

    #[test]
    fn documented_touchpad_report_rejects_one_byte_short_storage() {
        let bus = Bus::at_address(0x15, [descriptor(ELAN_DESCRIPTOR)]);
        let device = device_with_capacity(bus.clone(), 1128);

        assert!(matches!(
            block_on(device.read_report_descriptor()),
            Err(Error::Hid(hid::Error::InvalidSize(InvalidSizeError {
                expected: 1129,
                actual: 1128,
            })))
        ));
        bus.assert_finished();
    }

    #[test]
    fn cancelled_partial_metadata_refresh_refetches_before_next_command() {
        // Cancellation after a simulated partial DMA write must not publish the
        // partially overwritten buffer or reuse old command/data registers.
        // This exercises Device's software state, not a physical HAL's DMA/STOP.
        for refresh in [Refresh::Hid, Refresh::Report] {
            let bus = Bus::new([
                descriptor(BOOT_DESCRIPTOR),
                Transfer::WriteRead {
                    write: vec![1, 0],
                    reply: Reply::PartialPending(APP_DESCRIPTOR[..25].to_vec()),
                },
                descriptor(APP_DESCRIPTOR),
                write_read(&[0x34, 0x12, 0x31, 2, 0x78, 0x56], &[4, 0, 0xab, 0xcd]),
            ]);
            let device = device(bus.clone());
            assert_eq!(bytes(block_on(device.read_hid_descriptor()).unwrap()), BOOT_DESCRIPTOR);

            {
                let mut future = pin!(refresh.read(&device));
                let mut context = Context::from_waker(Waker::noop());
                assert!(matches!(future.as_mut().poll(&mut context), Poll::Pending));
            }

            let command = hid::Command::GetReport {
                report_type: hid::ReportType::Feature,
                report_id: hid::ReportId(1),
                expected_payload_size: Some(2),
            };
            let response = block_on(with_timeout(Duration::from_secs(1), device.handle_command(&command)))
                .expect("cancelled refresh retained the bus lock")
                .unwrap();
            let Some(Response::FeatureReport(response)) = response else {
                panic!("expected a feature report after cancellation");
            };
            assert_eq!(bytes(response), [4, 0, 0xab, 0xcd]);
            bus.assert_finished();
        }
    }

    #[test]
    fn cancelled_partial_report_is_replaced_by_complete_retry() {
        let report: Vec<u8> = (0..1129).map(|offset| (offset % 251) as u8).collect();
        let bus = Bus::at_address(
            0x15,
            [
                descriptor(ELAN_DESCRIPTOR),
                descriptor(ELAN_DESCRIPTOR),
                Transfer::WriteRead {
                    write: vec![2, 0],
                    reply: Reply::PartialPending(vec![0xa5; 17]),
                },
                descriptor(ELAN_DESCRIPTOR),
                write_read(&[2, 0], &report),
            ],
        );
        let device = device_with_capacity(bus.clone(), 1129);
        assert_eq!(bytes(block_on(device.read_hid_descriptor()).unwrap()), ELAN_DESCRIPTOR);

        {
            let mut future = pin!(device.read_report_descriptor());
            let mut context = Context::from_waker(Waker::noop());
            assert!(matches!(future.as_mut().poll(&mut context), Poll::Pending));
        }

        let response = block_on(with_timeout(Duration::from_secs(1), device.read_report_descriptor()))
            .expect("cancelled report retained the bus lock")
            .unwrap();
        assert_eq!(bytes(response), report);
        bus.assert_finished();
    }

    enum HostTransfer {
        Listen(SlaveCommand),
        FromHost(Vec<u8>),
        ToHost,
    }

    #[derive(Clone)]
    struct HostBus {
        transfers: Rc<RefCell<VecDeque<HostTransfer>>>,
        responses: Rc<RefCell<Vec<Vec<u8>>>>,
    }

    impl HostBus {
        fn new(transfers: impl IntoIterator<Item = HostTransfer>) -> Self {
            Self {
                transfers: Rc::new(RefCell::new(transfers.into_iter().collect())),
                responses: Rc::new(RefCell::new(Vec::new())),
            }
        }

        fn next(&self) -> HostTransfer {
            self.transfers
                .as_ref()
                .borrow_mut()
                .pop_front()
                .expect("unexpected upstream I2C operation")
        }
    }

    impl I2cSlaveAsync for HostBus {
        type Error = ErrorKind;

        async fn listen(&mut self) -> Result<SlaveCommand, Self::Error> {
            match self.next() {
                HostTransfer::Listen(command) => Ok(command),
                _ => panic!("expected upstream listen"),
            }
        }

        async fn respond_to_write(&mut self, buffer: &mut [u8]) -> Result<(), Self::Error> {
            match self.next() {
                HostTransfer::FromHost(data) => buffer.copy_from_slice(&data),
                _ => panic!("expected upstream write"),
            }
            Ok(())
        }

        async fn respond_to_read(&mut self, buffer: &[u8]) -> Result<(), Self::Error> {
            assert!(matches!(self.next(), HostTransfer::ToHost));
            self.responses.as_ref().borrow_mut().push(buffer.to_vec());
            Ok(())
        }
    }

    #[test]
    fn scripted_reenumeration_returns_current_descriptors_through_real_bridge() {
        // Guide §§2.1, 3.2 and 3.3 define application metadata and retrieval.
        // Version 2 and this host sequence are synthetic, NOT captured boot/app
        // descriptors or proof that an OS re-enumerates this way after update.
        let mut updated = ELAN_DESCRIPTOR;
        updated[24] = 2;
        let old_report = vec![0x11; 1129];
        let new_report = vec![0x22; 1129];
        let mut input = vec![0; 44];
        input[..3].copy_from_slice(&[44, 0, 0x52]);
        let downstream = Bus::at_address(
            0x15,
            [
                descriptor(ELAN_DESCRIPTOR),
                descriptor(ELAN_DESCRIPTOR),
                write_read(&[2, 0], &old_report),
                descriptor(updated),
                descriptor(updated),
                write_read(&[2, 0], &new_report),
                Transfer::Read(input.clone()),
                Transfer::Write(vec![5, 0, 0, 8]),
            ],
        );
        let mut transfers = Vec::new();
        for register in [[1, 0], [2, 0], [1, 0], [2, 0]] {
            transfers.extend([
                HostTransfer::Listen(SlaveCommand::Write),
                HostTransfer::FromHost(register.to_vec()),
                HostTransfer::Listen(SlaveCommand::Read),
                HostTransfer::ToHost,
            ]);
        }
        transfers.extend([
            HostTransfer::Listen(SlaveCommand::Read),
            HostTransfer::ToHost,
            HostTransfer::Listen(SlaveCommand::Write),
            HostTransfer::FromHost(vec![5, 0]),
            HostTransfer::FromHost(vec![0, 8]),
        ]);
        let upstream = HostBus::new(transfers);

        // Only this test registers endpoints: comms/HID registries are global
        // and have no unregister operation. Other tests use unregistered Device
        // instances. Keep all bridge scenarios in this one registration scope.
        let device: &'static Device<u8, Bus> = Box::leak(Box::new(device_with_capacity(downstream.clone(), 1129)));
        let host: &'static Host<HostBus> = Box::leak(Box::new(Host::new(
            hid::DeviceId(0),
            upstream.clone(),
            buffer(128),
            HostConfig::default(),
        )));
        block_on(async {
            embedded_services::init().await;
            hid::register_device(device).await.unwrap();
            comms::register_endpoint(host, &host.tp).await.unwrap();
        });

        let mut response_count = 0;
        for expected in [
            Some(ELAN_DESCRIPTOR.to_vec()),
            Some(old_report),
            Some(updated.to_vec()),
            Some(new_report),
            Some(input),
            None,
        ] {
            let (host_result, device_result) = block_on(with_timeout(
                Duration::from_secs(1),
                join(host.process(), device.process_request()),
            ))
            .expect("bridge did not complete the scripted request");
            host_result.unwrap();
            device_result.unwrap();

            // Observe actual bytes leaving Host before another request can
            // overwrite the shared buffer or hide an earlier stale response.
            let responses = upstream.responses.as_ref().borrow();
            if let Some(response) = expected {
                response_count += 1;
                assert_eq!(responses.last(), Some(&response));
            }
            assert_eq!(responses.len(), response_count);
        }

        assert!(upstream.transfers.as_ref().borrow().is_empty());
        downstream.assert_finished();
    }
}
