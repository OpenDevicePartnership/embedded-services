use crate::*;

use core::marker::PhantomData;
use embassy_time::{Duration, with_timeout};
use embedded_mcu_hal::i2c::target::asynch::I2c as I2cTargetAsync;
use embedded_mcu_hal::i2c::target::{ReadStatus, Request, WriteStatus};
use embedded_services::relay::hid;
use embedded_services::relay::hid::{GetHidReportType, HidError, HidReport, SetHidReport};
use zerocopy::IntoBytes;

/// HID-I2C Command Opcode as specified in section 7.1.1 of the HID-I2C spec
#[repr(u8)]
#[derive(num_enum::TryFromPrimitive, num_enum::IntoPrimitive, Debug, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
enum Opcode {
    // Reserved: 0x00
    Reset = 0x01,
    GetReport = 0x02,
    SetReport = 0x03,

    // The following commands are in the i2c spec but are listed as optional and
    // "not sent by modern hosts", and therefore we do not implement them:
    //
    // GetIdle = 0x04,
    // SetIdle = 0x05,
    // GetProtocol = 0x06,
    // SetProtocol = 0x07,
    SetPower = 0x08,
    // Reserved: 0x09 - 0x0D
    // VendorReserved = 0x0E,
    // Reserved: 0x0F
}

/// I2C wire format representation for HID power states.
#[repr(u8)]
#[derive(num_enum::TryFromPrimitive, num_enum::IntoPrimitive, Debug, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
enum I2cPowerState {
    On = 0x00,
    Sleep = 0x01,
}

impl From<I2cPowerState> for hid::HidDevicePowerState {
    fn from(value: I2cPowerState) -> Self {
        match value {
            I2cPowerState::On => hid::HidDevicePowerState::On,
            I2cPowerState::Sleep => hid::HidDevicePowerState::Sleep,
        }
    }
}

#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
enum HidI2cReportType {
    Input,
    Output,
    Feature,
}

impl TryFrom<HidI2cReportType> for GetHidReportType {
    type Error = ProtocolError;

    fn try_from(value: HidI2cReportType) -> Result<Self, Self::Error> {
        match value {
            HidI2cReportType::Input => Ok(GetHidReportType::Input),
            HidI2cReportType::Feature => Ok(GetHidReportType::Feature),
            HidI2cReportType::Output => Err(ProtocolError::InvalidReportType),
        }
    }
}

struct HidI2cReportCommandHeader {
    /// The report type that this command is targeting
    report_type: HidI2cReportType,

    /// The report ID that this command is targeting, or None if another byte must be read to get the full report ID (happens if report ID is >= 0xF)
    report_id: Option<embedded_services::relay::hid::ReportId>,
}

impl HidI2cReportCommandHeader {
    fn try_from_command_byte(command_byte: u8) -> Result<Self, ProtocolError> {
        const HID_I2C_REPORT_TYPE_OFFSET: u8 = 4;
        let report_type = match command_byte >> HID_I2C_REPORT_TYPE_OFFSET {
            0x01 => HidI2cReportType::Input,
            0x02 => HidI2cReportType::Output,
            0x03 => HidI2cReportType::Feature,
            _ => return Err(ProtocolError::InvalidReportType),
        };
        let report_id = if command_byte & 0x0F == 0x0F {
            None
        } else {
            Some(embedded_services::relay::hid::ReportId(command_byte & 0x0F))
        };
        Ok(Self { report_type, report_id })
    }
}

/// Resources used by the service
struct InnerResources {
    reset_signal: embassy_sync::signal::Signal<embedded_services::GlobalRawMutex, ()>,
}

/// Memory required for the HID-I2C target service.
pub struct Resources<Bus: I2cTargetAsync, AttnPin: embedded_hal::digital::OutputPin, HidDevice: ConstrainedHidDevice> {
    inner: Option<InnerResources>,

    // We don't currently need these to be shared between the runner and the service, but we may in the future,
    // and being generic over them now means that we can move stuff in here later without a breaking interface change.
    _phantom: PhantomData<(Bus, AttnPin, HidDevice)>,
}

impl<Bus: I2cTargetAsync, AttnPin: embedded_hal::digital::OutputPin, HidDevice: ConstrainedHidDevice> Default
    for Resources<Bus, AttnPin, HidDevice>
{
    fn default() -> Self {
        Self {
            inner: None,
            _phantom: PhantomData,
        }
    }
}

/// Wrapper for the I2C trait that automatically handles timeouts and recovery
struct TimeoutBus<Bus: I2cTargetAsync> {
    bus: Bus,

    timeout_settings: TimeoutSettings,
}

impl<Bus: I2cTargetAsync> TimeoutBus<Bus> {
    /// Wait for the next controller-initiated event with no timeout.
    fn listen_indefinitely(&mut self) -> impl core::future::Future<Output = Result<Request, Bus::Error>> + '_ {
        self.bus.listen()
    }

    /// Wait for the controller to address us mid-transaction, applying the device-response timeout
    /// and skipping repeated-start edges.
    async fn listen_for_response(&mut self) -> Result<Request, Error<Bus::Error>> {
        loop {
            let result = with_timeout(self.timeout_settings.device_response_timeout, self.bus.listen()).await?;
            let result = result.map_err(Error::Bus)?;
            if let Request::RepeatedStart(_a) = result {
                continue;
            }

            return Ok(result);
        }
    }

    /// Read bytes the host is writing to us, applying the data-read timeout and recovering the bus on failure.
    /// Buffer must be as large as the largest possible write the host can do in a single transaction. If the host
    /// writes more bytes than the provided buffer, we drop any remaining bytes so as to not stall the bus and return
    /// an error.
    async fn read<'buf>(&mut self, buffer: &'buf mut [u8]) -> Result<&'buf [u8], Error<Bus::Error>> {
        match with_timeout(
            self.timeout_settings.data_read_timeout,
            self.bus.respond_to_write(buffer),
        )
        .await
        {
            // Timed out waiting for the controller to drive the transfer.
            Err(_timeout_error) => {
                error!("Read request timeout");
                self.bus.recover().await.map_err(Error::Bus)?;
                Err(Error::Protocol(ProtocolError::Timeout))
            }
            // Controller finished writing; report how many bytes we drained.
            Ok(Ok(status @ (WriteStatus::Stopped(bytes) | WriteStatus::Restarted(bytes)))) => {
                trace!("Host issued write command: {:?}", status);

                Ok(buffer.get(..bytes).ok_or(Error::Protocol(ProtocolError::InvalidData))?)
            }
            Ok(Ok(WriteStatus::BufferFull(_bytes))) => {
                warn!("Host attempted to issue more bytes than we can handle - failing read");
                self.discard_remaining_bytes_from_host().await?;
                Err(Error::Protocol(ProtocolError::InvalidData))
            }
            // Some other write status we don't expect while reading.
            Ok(Ok(status)) => {
                error!("Unexpected write status: {:?}", status);
                Err(Error::Protocol(ProtocolError::InvalidData))
            }
            // The bus peripheral itself reported an error.
            Ok(Err(e)) => {
                error!("Error during bus read");
                Err(Error::Bus(e))
            }
        }
    }

    async fn discard_remaining_bytes_from_host(&mut self) -> Result<(), Error<Bus::Error>> {
        let mut discard_buffer = [0u8; 16];
        loop {
            let result = with_timeout(
                self.timeout_settings.data_read_timeout,
                self.bus.respond_to_write(&mut discard_buffer),
            )
            .await?
            .map_err(Error::Bus)?;
            match result {
                WriteStatus::BufferFull(_bytes) => {
                    continue;
                }
                _ => return Ok(()),
            }
        }
    }

    /// Write all of `buffer` to the host, padding with zeros if the host asks for more bytes.
    async fn write(&mut self, buffer: &[u8]) -> Result<(), Error<Bus::Error>> {
        let mut write_buffer: &[u8] = buffer;
        const PADDING_BUFFER: &[u8] = &[0u8; 8];
        while self.write_unterminated(write_buffer).await? {
            write_buffer = PADDING_BUFFER;
            trace!("Emitting a padding byte");
        }
        Ok(())
    }

    /// Write `buffer` to the host; returns true if the host requested more bytes than we provided.
    /// TODO - we should augment the I2C trait to allow us to write a slice of slices in a single operation so we don't have
    ///        multiple await points, which causes us to hog the bus.  When we land that, remove this and switch to that API instead.
    async fn write_unterminated(&mut self, buffer: &[u8]) -> Result<bool, Error<Bus::Error>> {
        match with_timeout(
            self.timeout_settings.device_response_timeout,
            self.bus.respond_to_read(buffer),
        )
        .await
        {
            Err(_timeout_error) => {
                error!("Write request timeout");
                self.bus.recover().await.map_err(Error::Bus)?;
                Err(Error::Protocol(ProtocolError::Timeout))
            }
            Ok(result) => result
                .map(|read_status| match read_status {
                    ReadStatus::NeedMore(_) => {
                        trace!("host requested more bytes than we provided");
                        true
                    }
                    _ => false,
                })
                .map_err(Error::Bus),
        }
    }
}

/// Service runner for the HID-I2C service. You must call run() on the runner to drive the service.
pub struct Runner<'hw, Bus: I2cTargetAsync, AttnPin: embedded_hal::digital::OutputPin, HidDevice: ConstrainedHidDevice>
{
    bus: TimeoutBus<Bus>,
    attn_pin: AttnPinHandler<AttnPin>,
    hid_device: HidDevice,
    device_descriptor: DeviceDescriptor,

    /// Buffer for receiving messages.
    write_buf: generic_array::GenericArray<u8, HidDevice::WriteBufferSize>,

    /// True if a reset has been triggered but not yet acknowledged by the host
    pending_reset: bool,

    resources: &'hw InnerResources,
}

impl<
    'hw,
    Bus: I2cTargetAsync + 'hw,
    AttnPin: embedded_hal::digital::OutputPin + 'hw,
    HidDevice: ConstrainedHidDevice + 'hw,
> odp_service_common::runnable_service::ServiceRunner<'hw> for Runner<'hw, Bus, AttnPin, HidDevice>
{
    async fn run(mut self) -> embedded_services::Never {
        loop {
            let event = {
                // If we've raised the interrupt, we know it won't be dismissed again until it's serviced by the host reading
                // the input report, so we don't need to listen for another notification.
                let input_report_ready_future = async {
                    if self.attn_pin.asserted() {
                        core::future::pending().await
                    } else {
                        self.hid_device.wait_for_input_report().await
                    }
                };
                embassy_futures::select::select3(
                    self.bus.listen_indefinitely(),
                    input_report_ready_future,
                    self.resources.reset_signal.wait(),
                )
                .await
            };
            match event {
                embassy_futures::select::Either3::First(bus_request) => {
                    trace!("HID-I2C: Processing request from host");
                    match bus_request {
                        Ok(request) => {
                            self.process_request(request).await;
                        }
                        Err(bus_error) => {
                            error!(
                                "HID-I2C: Error during bus operation: {:?}",
                                embedded_mcu_hal::i2c::target::Error::kind(&bus_error)
                            );
                        }
                    }
                }
                embassy_futures::select::Either3::Second(()) => {
                    trace!("HID-I2C: Signalling host that an input report is ready");
                    self.attn_pin.assert_interrupt();
                }
                embassy_futures::select::Either3::Third(()) => {
                    trace!("HID-I2C: Received reset request");
                    self.reset().await;
                }
            }
        }
    }
}

impl<
    'hw,
    Bus: I2cTargetAsync + 'hw,
    AttnPin: embedded_hal::digital::OutputPin + 'hw,
    HidDevice: ConstrainedHidDevice + 'hw,
> Runner<'hw, Bus, AttnPin, HidDevice>
{
    async fn process_request(&mut self, request: Request) {
        // TODO unlike the old trait where the address was fixed, this one can get multiple addresses.
        //      We may need to have some way to split the bus resources across multiple logical I2C devices,
        //      perhaps some sort of "I2cSocket" abstraction built on top of the I2cTargetAsync trait that can
        //      be used to scope the addressing to a single device or something.
        //
        //      For now, assume that there's only one address on the bus and it's us. This will explode spectacularly
        //      if that's not the case, though, so we'll need to revisit this at some point.
        //
        let result = match request {
            Request::Write(_address) => {
                trace!("HID-I2C: Processing register access");
                self.process_register_access().await
            }
            Request::Read(_address) => {
                trace!("HID-I2C: Processing request for input report");
                self.reply_with_input_report().await
            }
            _ => {
                trace!("HID-I2C: Ignoring command type {:?}", request);
                return;
            }
        };

        match result {
            Ok(_) => {}
            Err(Error::Bus(bus_error)) => {
                error!(
                    "HID-I2C: Error during bus operation: {:?}",
                    embedded_mcu_hal::i2c::target::Error::kind(&bus_error)
                );
            }
            Err(Error::Protocol(protocol_error)) => {
                error!("HID-I2C: Protocol error during bus operation: {:?}", protocol_error);
            }
            Err(Error::Device(HidError::TriggerReset)) => {
                warn!("HID-I2C: HID device requested device-initiated reset");
                self.reset().await;
            }
            Err(Error::Device(hid_error)) => {
                error!(
                    "HID-I2C: non-resetting HID device error during bus operation: {:?}",
                    hid_error
                );
            }
        }
    }

    async fn process_register_access(&mut self) -> Result<(), Error<Bus::Error>> {
        let data = self.bus.read(&mut self.write_buf).await?;

        let (&register, data) = data
            .split_first_chunk::<2>()
            .ok_or(Error::Protocol(ProtocolError::InvalidData))?;

        let register = HidI2cRegister::try_from(u16::from_le_bytes(register))
            .map_err(|_| Error::Protocol(ProtocolError::InvalidRegisterAddress))?;

        info!("HID-I2C: Host requested to access register {:?}", register);
        match register {
            HidI2cRegister::DeviceDescriptor => {
                let request = self.bus.listen_for_response().await?;
                match request {
                    Request::Read(_address) => {
                        trace!(
                            "Responding to request for device descriptor with {} bytes",
                            self.device_descriptor.as_bytes().len()
                        );
                        self.bus.write(self.device_descriptor.as_bytes()).await?;

                        Ok(())
                    }
                    _ => {
                        error!(
                            "Expected read request after device descriptor register access: {:?}",
                            request
                        );
                        Err(Error::Protocol(ProtocolError::InvalidRegisterAddress))
                    }
                }
            }
            HidI2cRegister::ReportDescriptor => match self.bus.listen_for_response().await? {
                Request::Read(_address) => {
                    trace!("Responding to request for report descriptor");
                    self.bus.write(self.hid_device.report_descriptor().as_bytes()).await?;
                    Ok(())
                }
                _ => {
                    error!("Expected read request after report descriptor register access");
                    Err(Error::Protocol(ProtocolError::InvalidRegisterAddress))
                }
            },
            HidI2cRegister::Input => self.process_input_report_read().await,
            HidI2cRegister::Output => {
                let (&report_length_bytes, data) = data
                    .split_first_chunk::<{ core::mem::size_of::<u16>() }>()
                    .ok_or(Error::Protocol(ProtocolError::InvalidSize))?;

                // NOTE - if we're using implicit report IDs, we present that to HidDevice implementations as report ID 0.
                let (&report_id, data) = if self.hid_device.report_descriptor().report_ids_implicit() {
                    (&0, data)
                } else {
                    data.split_first().ok_or(Error::Protocol(ProtocolError::InvalidSize))?
                };

                let header_size_bytes = 2 + if self.hid_device.report_descriptor().report_ids_implicit() {
                    0
                } else {
                    1
                };

                let length = (u16::from_le_bytes(report_length_bytes) as usize)
                    .checked_sub(header_size_bytes)
                    .ok_or(Error::Protocol(ProtocolError::InvalidSize))?; // Note: per HID spec, the length field needs to include its own length (2 bytes) and the report ID (1 byte)

                let output_report = embedded_services::relay::hid::SetHidReport::Output(HidReport::new(
                    embedded_services::relay::hid::ReportId(report_id),
                    data.get(..length).ok_or(Error::Protocol(ProtocolError::InvalidSize))?,
                ));

                self.hid_device.set_report(&output_report).await?;

                Ok(())
            }
            HidI2cRegister::Command => Self::process_command(data, &mut self.bus, &mut self.hid_device).await,
            HidI2cRegister::Data => {
                error!(
                    "HID-I2C: Got read to Data register without a preceding write to the Command register; this is unexpected and may indicate a bug in the service."
                );
                Err(Error::Protocol(ProtocolError::InvalidRegisterAddress))
            }
        }
    }

    /// Process a request for an input report that we've asserted an interrupt for (i.e. not a request for a specific input report ID)
    async fn process_input_report_read(&mut self) -> Result<(), Error<Bus::Error>> {
        info!("Processing normal input report request");
        let read_request = self.bus.listen_for_response().await?;
        if let Request::Read(_address) = read_request {
            self.reply_with_input_report().await
        } else {
            error!(
                "Expected read request after input report register access, got {:?}",
                read_request
            );
            Err(Error::Protocol(ProtocolError::InvalidCommand))
        }
    }

    // Respond to the host with the next input report.
    async fn reply_with_input_report(&mut self) -> Result<(), Error<Bus::Error>> {
        if self.pending_reset {
            info!("HID-I2C: Processing first input report read after reset");
            // We need to acknowledge that we've completed a reset by writing back 0's - see section 7.2.1 of the HID spec
            self.bus.write(&[00, 00]).await?;

            self.pending_reset = false;
            self.attn_pin.clear_interrupt();
            return Ok(());
        }

        // If the host reads the input register when we have no report queued, return an empty report.
        // In general, this should not happen (the host should only poll us when we've asserted the interrupt,
        // which we only do when we have a report ready), but if it does due to e.g. a host-side race condition,
        // we'll stall the I2C bus if we don't respond.
        //
        if !self.hid_device.has_pending_input_report() {
            warn!("HID-I2C: Host polled when no input report was pending; responding with zero-length report");
            self.bus.write(&[00, 00]).await?;
            self.attn_pin.clear_interrupt();
            return Ok(());
        }

        let report_ids_implicit = self.hid_device.report_descriptor().report_ids_implicit();
        self.hid_device
            .process_next_input_report(async |report| {
                let size_bytes = report.data().len() as u16
                    + device_descriptor::HID_REPORT_HEADER_SIZE_BYTES
                    + if report_ids_implicit {
                        0
                    } else {
                        device_descriptor::HID_REPORT_ID_SIZE_BYTES
                    };
                let [size_low, size_high] = size_bytes.to_le_bytes();

                let header_slice: &[u8] = if report_ids_implicit {
                    &[size_low, size_high]
                } else {
                    &[size_low, size_high, report.id().0]
                };

                self.bus.write_unterminated(header_slice).await?;
                self.bus.write(report.data()).await?;
                Ok::<(), Error<Bus::Error>>(())
            })
            .await??;

        if !self.hid_device.has_pending_input_report() {
            self.attn_pin.clear_interrupt();
        }

        Ok(())
    }

    /// Attempts to parse the provided command byte as an input / output command header, including consuming any additional
    /// bytes from the read as needed.
    /// Returns:
    /// - The report type (input, output, feature)
    /// - The report ID
    /// - A slice over the remaining bytes from data
    async fn get_io_command_report_header(
        data: &[u8],
        command_byte: u8,
    ) -> Result<(HidI2cReportType, embedded_services::relay::hid::ReportId, &[u8]), Error<Bus::Error>> {
        let command_header = HidI2cReportCommandHeader::try_from_command_byte(command_byte)?;
        let (report_id, data) = if let Some(report_id) = command_header.report_id {
            (report_id, data)
        } else {
            let (&report_id, data) = data.split_first().ok_or(Error::Protocol(ProtocolError::InvalidSize))?;
            (embedded_services::relay::hid::ReportId(report_id), data)
        };

        let (&data_register_address, data) = data
            .split_first_chunk::<{ core::mem::size_of::<u16>() }>()
            .ok_or(Error::Protocol(ProtocolError::InvalidSize))?;

        let data_register_address = u16::from_le_bytes(data_register_address);
        if data_register_address != HidI2cRegister::Data as u16 {
            error!(
                "Expected the host to write the data register address after the header ({:?}) but got {:?}",
                HidI2cRegister::Data,
                data_register_address
            );
            return Err(Error::Protocol(ProtocolError::InvalidRegisterAddress));
        }

        Ok((command_header.report_type, report_id, data))
    }

    async fn process_command(
        data: &[u8],
        bus: &mut TimeoutBus<Bus>,
        hid_device: &mut HidDevice,
    ) -> Result<(), Error<Bus::Error>> {
        let (&command_byte, data) = data
            .split_first()
            .ok_or(Error::Protocol(ProtocolError::InvalidCommand))?;
        let (&opcode_byte, data) = data
            .split_first()
            .ok_or(Error::Protocol(ProtocolError::InvalidCommand))?;

        match Opcode::try_from(opcode_byte).map_err(|_| Error::Protocol(ProtocolError::InvalidCommand))? {
            Opcode::Reset => {
                warn!("HID-I2C: Host requested device reset");
                Err(Error::Device(HidError::TriggerReset))
            }

            Opcode::SetPower => {
                trace!("Processing set power command");
                let power_state = I2cPowerState::try_from(command_byte)
                    .map_err(|_| Error::Protocol(ProtocolError::InvalidCommand))?;
                hid_device.set_power_state(power_state.into()).await?;
                Ok(())
            }

            Opcode::GetReport => {
                trace!("Processing get report command");

                let (report_type, report_id, _data) = Self::get_io_command_report_header(data, command_byte).await?;

                // TODO - here, if the report ID is invalid, we're supposed to return a zero-length report.  We should know from the
                //        report descriptor whether the report ID is valid or not, but we don't yet have the report descriptor parsing
                //        implemented, so we can't do that yet.  For now, that responsibility has to fall on the HidDevice implementation,
                //        but as soon as the aggregation / HID library goes in, look into leveraging it for filtering out invalid report
                //        IDs here.

                hid_device
                    .process_get_report(report_type.try_into()?, report_id, async |report| {
                        // Note: per HID spec, the length field needs to include its own length (2 bytes)
                        let len_header = (report.data().len() as u16 + device_descriptor::HID_REPORT_HEADER_SIZE_BYTES)
                            .to_le_bytes();
                        bus.write_unterminated(&len_header).await?;
                        bus.write(report.data()).await?;
                        Ok::<(), Error<Bus::Error>>(())
                    })
                    .await??;

                Ok(())
            }

            Opcode::SetReport => {
                trace!("Processing set report command");
                let (report_type, report_id, data) = Self::get_io_command_report_header(data, command_byte).await?;

                let (&len_header, data) = data
                    .split_first_chunk::<{ core::mem::size_of::<u16>() }>()
                    .ok_or(Error::Protocol(ProtocolError::InvalidSize))?;

                // Note: per HID spec, the length field relayed over the wire needs to include its own length (2 bytes)
                let report_size = (u16::from_le_bytes(len_header)
                    .checked_sub(device_descriptor::HID_REPORT_HEADER_SIZE_BYTES))
                .ok_or(Error::Protocol(ProtocolError::InvalidSize))? as usize;

                let report_data = data
                    .get(..report_size)
                    .ok_or(Error::Protocol(ProtocolError::InvalidSize))?;

                let set_report = match report_type {
                    HidI2cReportType::Input => {
                        error!("Host attempted to send us an input report, which is invalid");
                        return Err(Error::Protocol(ProtocolError::InvalidReportType));
                    }
                    HidI2cReportType::Output => SetHidReport::Output(HidReport::new(report_id, report_data)),
                    HidI2cReportType::Feature => SetHidReport::Feature(HidReport::new(report_id, report_data)),
                };

                hid_device.set_report(&set_report).await?;

                Ok(())
            }
        }
    }

    async fn reset(&mut self) {
        warn!("HID-I2C: Executing device reset");
        self.hid_device.reset().await;
        self.pending_reset = true;
        self.attn_pin.assert_interrupt();
    }
}

/// Control handle for an instance of the HID-I2C service, which presents a HID-I2C device over an (I2C bus, interrupt line) tuple
#[derive(Clone, Copy)]
pub struct Service<'hw, Bus: I2cTargetAsync, AttnPin: embedded_hal::digital::OutputPin, HidDevice: ConstrainedHidDevice>
{
    resources: &'hw InnerResources,
    _phantom: core::marker::PhantomData<(Bus, AttnPin, HidDevice)>,
}

impl<
    'hw,
    Bus: I2cTargetAsync + 'hw,
    AttnPin: embedded_hal::digital::OutputPin + 'hw,
    HidDevice: ConstrainedHidDevice + 'hw,
> Service<'hw, Bus, AttnPin, HidDevice>
{
    /// Creates a new instance of the HID-I2C service and its associated runner.
    /// You must call run() on the runner to drive the service.  Consider using
    /// this in conjunction with `odp_service_common::runnable_service::spawn_service!()`
    pub async fn new(
        storage: &'hw mut Resources<Bus, AttnPin, HidDevice>,
        bus: Bus,
        attn_pin: AttnPin,
        hid_device: HidDevice,
        hwinfo: HardwareVersionInfo,
        timeout_settings: TimeoutSettings,
    ) -> Result<(Self, Runner<'hw, Bus, AttnPin, HidDevice>), core::convert::Infallible> {
        let device_descriptor = DeviceDescriptor::new(&hid_device, hwinfo);

        let resources = storage.inner.insert(InnerResources {
            reset_signal: embassy_sync::signal::Signal::new(),
        });

        Ok((
            Service {
                resources,
                _phantom: PhantomData,
            },
            Runner {
                bus: TimeoutBus { bus, timeout_settings },
                attn_pin: AttnPinHandler::new(attn_pin),
                hid_device,
                device_descriptor,
                write_buf: generic_array::GenericArray::default(),
                pending_reset: false, // The host is responsible for explicitly resetting us at boot, so we start in a non-reset state
                resources,
            },
        ))
    }

    /// Causes the HID service to perform a device-initiated reset.
    pub fn reset(&mut self) {
        self.resources.reset_signal.signal(());
    }
}

impl<
    'hw,
    Bus: I2cTargetAsync + 'hw,
    AttnPin: embedded_hal::digital::OutputPin + 'hw,
    HidDevice: ConstrainedHidDevice + 'hw,
> odp_service_common::runnable_service::Service<'hw> for Service<'hw, Bus, AttnPin, HidDevice>
{
    type Runner = Runner<'hw, Bus, AttnPin, HidDevice>;
    type Resources = Resources<Bus, AttnPin, HidDevice>;
}

/// Timeout configuration for I2C operations
pub struct TimeoutSettings {
    /// Timeout for device response reads
    pub device_response_timeout: Duration,
    /// Timeout for data reads from the host.
    pub data_read_timeout: Duration,
}

impl Default for TimeoutSettings {
    fn default() -> Self {
        Self {
            device_response_timeout: Duration::from_secs(1),
            data_read_timeout: Duration::from_secs(1),
        }
    }
}
