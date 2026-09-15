use crate::*;

use crate::wire::{Command, ReportFraming, ReportHeader, SetReportType};
use core::marker::PhantomData;
use embassy_time::{Duration, with_timeout};
use embedded_mcu_hal::i2c::target::asynch::I2c as I2cTargetAsync;
use embedded_mcu_hal::i2c::target::{ReadStatus, Request, WriteStatus};
use embedded_services::relay::hid::{HidError, SetHidReport};
use zerocopy::IntoBytes;

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
            .await;

            let Ok(result) = result else {
                self.bus.recover().await.map_err(Error::Bus)?;
                return Err(Error::Protocol(ProtocolError::Timeout));
            };

            match result.map_err(Error::Bus)? {
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
                    self.attn_pin
                        .assert_interrupt()
                        .unwrap_or_else(|_| error!("HID-I2C: Failed to assert interrupt on attn pin"));
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
                // The Output register carries a report in the same framing as SET_REPORT
                // (spec sections 6.2.2 and 7.2.3.1), so the same parser handles both.
                let framing = ReportFraming::of(self.hid_device.report_descriptor());
                let output_report = SetHidReport::Output(wire::parse_report(data, framing)?);

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
            self.attn_pin
                .clear_interrupt()
                .unwrap_or_else(|_| error!("HID-I2C: Failed to clear interrupt on attn pin"));
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
            self.attn_pin
                .clear_interrupt()
                .unwrap_or_else(|_| error!("HID-I2C: Failed to clear interrupt on attn pin"));
            return Ok(());
        }

        let framing = ReportFraming::of(self.hid_device.report_descriptor());
        self.hid_device
            .process_next_input_report(async |report| {
                let header = ReportHeader::new(report.data().len(), report.id(), framing)?;

                self.bus.write_unterminated(header.as_bytes()).await?;
                self.bus.write(report.data()).await?;
                Ok::<(), Error<Bus::Error>>(())
            })
            .await??;

        if !self.hid_device.has_pending_input_report() {
            self.attn_pin
                .clear_interrupt()
                .unwrap_or_else(|_| error!("HID-I2C: Failed to clear interrupt on attn pin"));
        }

        Ok(())
    }

    /// Handle a command written to the Command register (spec section 7.2).
    ///
    /// All parsing happens up front in [`Command::parse`]; everything below the parse operates
    /// on values that cannot be malformed, so this function contains only I/O and dispatch.
    async fn process_command(
        data: &[u8],
        bus: &mut TimeoutBus<Bus>,
        hid_device: &mut HidDevice,
    ) -> Result<(), Error<Bus::Error>> {
        let framing = ReportFraming::of(hid_device.report_descriptor());

        match Command::parse(data, framing)? {
            Command::Reset => {
                warn!("HID-I2C: Host requested device reset");
                Err(Error::Device(HidError::TriggerReset))
            }

            Command::SetPower(power_state) => {
                trace!("Processing set power command");
                hid_device.set_power_state(power_state.into()).await?;
                Ok(())
            }

            Command::GetReport { report_type, report_id } => {
                trace!("Processing get report command");

                // TODO - here, if the report ID is invalid, we're supposed to return a zero-length report
                //        (spec section 7.2.2.2).  We should know from the report descriptor whether the
                //        report ID is valid or not, but we don't yet have the report descriptor parsing
                //        implemented, so we can't do that yet.  For now, that responsibility has to fall
                //        on the HidDevice implementation, but as soon as the aggregation / HID library
                //        goes in, look into leveraging it for filtering out invalid report IDs here.

                match bus.listen_for_response().await? {
                    Request::Read(_address) => {}
                    other => {
                        error!("Expected read request after get report command, got {:?}", other);
                        return Err(Error::Protocol(ProtocolError::InvalidCommand));
                    }
                }

                hid_device
                    .process_get_report(report_type.into(), report_id, async |report| {
                        let header = ReportHeader::new(report.data().len(), report_id, framing)?;

                        bus.write_unterminated(header.as_bytes()).await?;
                        bus.write(report.data()).await?;
                        Ok::<(), Error<Bus::Error>>(())
                    })
                    .await??;

                Ok(())
            }

            Command::SetReport { report_type, report } => {
                trace!("Processing set report command");

                let set_report = match report_type {
                    SetReportType::Output => SetHidReport::Output(report),
                    SetReportType::Feature => SetHidReport::Feature(report),
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
        self.attn_pin
            .assert_interrupt()
            .unwrap_or_else(|_| error!("HID-I2C: Failed to assert interrupt on attn pin"));
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
    ) -> Result<(Self, Runner<'hw, Bus, AttnPin, HidDevice>), crate::DeviceDescriptorError> {
        let device_descriptor = DeviceDescriptor::new(&hid_device, hwinfo)?;

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

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::test_support::{PinLevel, RecordingHidDevice, RecordingPin, hardware_version_info, recording_device};
    use crate::wire::Opcode;
    use embedded_mcu_hal::i2c::target::{ErrorKind, ErrorType, ReadStatus, WriteStatus};
    use embedded_services::relay::hid::{HidDevicePowerState, ReportId};
    use std::collections::VecDeque;

    /// Bus error used by the mocks. Deliberately not `Infallible`: every `Error::Bus` arm in the
    /// service is unreachable by construction if the mock cannot fail.
    type MockBusError = ErrorKind;

    /// A bus that never produces an event, so every wait against it hits the caller's timeout.
    struct NoopBus;

    impl ErrorType for NoopBus {
        type Error = MockBusError;
    }

    impl I2cTargetAsync for NoopBus {
        async fn recover(&mut self) -> Result<(), Self::Error> {
            Ok(())
        }

        async fn listen(&mut self) -> Result<Request, Self::Error> {
            core::future::pending().await
        }

        async fn respond_to_read(&mut self, _buf: &[u8]) -> Result<ReadStatus, Self::Error> {
            core::future::pending().await
        }

        async fn respond_to_write(&mut self, _buf: &mut [u8]) -> Result<WriteStatus, Self::Error> {
            core::future::pending().await
        }
    }

    fn timeout_bus() -> TimeoutBus<NoopBus> {
        TimeoutBus {
            bus: NoopBus,
            timeout_settings: TimeoutSettings::default(),
        }
    }

    struct IncomingWrite {
        data: Vec<u8>,
        status: WriteStatus,
    }

    #[derive(Default)]
    struct ScriptedBus {
        /// Controller-initiated events handed out by `listen()`, in order. When empty, `listen()`
        /// stays pending so the caller's timeout fires.
        listen_requests: VecDeque<Request>,
        incoming_writes: VecDeque<IncomingWrite>,
        read_statuses: VecDeque<ReadStatus>,
        outgoing_reads: Vec<Vec<u8>>,
        recover_count: usize,
        /// When set, the next `respond_to_read` fails with this error.
        fail_next_read: Option<MockBusError>,
        /// When set, the next `respond_to_write` fails with this error.
        fail_next_write: Option<MockBusError>,
        /// When set, `recover()` fails with this error.
        fail_recover: Option<MockBusError>,
    }

    impl ErrorType for ScriptedBus {
        type Error = MockBusError;
    }

    impl I2cTargetAsync for ScriptedBus {
        async fn recover(&mut self) -> Result<(), Self::Error> {
            self.recover_count += 1;
            match self.fail_recover {
                Some(error) => Err(error),
                None => Ok(()),
            }
        }

        async fn listen(&mut self) -> Result<Request, Self::Error> {
            let Some(request) = self.listen_requests.pop_front() else {
                return core::future::pending().await;
            };
            Ok(request)
        }

        async fn respond_to_read(&mut self, buf: &[u8]) -> Result<ReadStatus, Self::Error> {
            if let Some(error) = self.fail_next_read.take() {
                return Err(error);
            }
            let Some(status) = self.read_statuses.pop_front() else {
                return core::future::pending().await;
            };
            self.outgoing_reads.push(buf.to_vec());
            Ok(status)
        }

        async fn respond_to_write(&mut self, buf: &mut [u8]) -> Result<WriteStatus, Self::Error> {
            if let Some(error) = self.fail_next_write.take() {
                return Err(error);
            }
            let Some(write) = self.incoming_writes.pop_front() else {
                return core::future::pending().await;
            };
            for (destination, source) in buf.iter_mut().zip(write.data.iter()) {
                *destination = *source;
            }
            Ok(write.status)
        }
    }

    fn scripted_timeout_bus(bus: ScriptedBus) -> TimeoutBus<ScriptedBus> {
        TimeoutBus {
            bus,
            timeout_settings: TimeoutSettings {
                device_response_timeout: Duration::from_millis(20),
                data_read_timeout: Duration::from_millis(20),
            },
        }
    }

    // Command-byte and frame parsing is covered by the pure tests in `crate::wire`, which need
    // no bus, no pin, no device and no async runtime. The tests here exercise the I/O shell.

    #[tokio::test]
    async fn set_power_command_updates_device() {
        let mut bus = timeout_bus();
        let mut device = recording_device();

        Runner::<NoopBus, RecordingPin, RecordingHidDevice>::process_command(
            // Command register (little-endian): low byte 0x01 = power state Sleep, high byte = SetPower opcode.
            &[0x01, Opcode::SetPower as u8],
            &mut bus,
            &mut device,
        )
        .await
        .unwrap();

        assert!(matches!(device.power_state, Some(HidDevicePowerState::Sleep)));
    }

    #[tokio::test]
    async fn reset_command_requests_device_reset() {
        let mut bus = timeout_bus();
        let mut device = recording_device();

        let result = Runner::<NoopBus, RecordingPin, RecordingHidDevice>::process_command(
            // Command register (little-endian): low byte is unused for Reset, high byte = Reset opcode.
            &[0x00, Opcode::Reset as u8],
            &mut bus,
            &mut device,
        )
        .await;

        assert!(matches!(result, Err(Error::Device(HidError::TriggerReset))));
    }

    #[tokio::test]
    async fn set_feature_report_accepts_extended_report_id() {
        let mut bus = timeout_bus();
        let mut device = recording_device();
        let command = [
            0x3f,                       // command low byte: report type Feature (0x3), report ID nibble 0xF = extended
            Opcode::SetReport as u8,    // command high byte: SetReport opcode
            0x21,                       // extended report ID (0x21)
            HidI2cRegister::Data as u8, // data register address, low byte (0x06)
            0x00,                       // data register address, high byte -> 0x0006
            0x04,                       // wLength low byte: 2 (self) + 1 (report ID) + 1 (payload)
            0x00,                       // wLength high byte
            0x21,                       // report ID, repeated in the data payload per spec 7.2.3.1
            0x5a,                       // report payload
        ];

        Runner::<NoopBus, RecordingPin, RecordingHidDevice>::process_command(&command, &mut bus, &mut device)
            .await
            .unwrap();

        assert_eq!(device.report_id, Some(ReportId(0x21)));
        assert_eq!(device.report_data.get(..device.report_len), Some(&[0x5a][..]));
        assert!(device.feature_report);
    }

    /// The report ID appears twice in a `SET_REPORT`: in the command header and again in the data
    /// payload. A host that disagrees with itself is a protocol violation, not a report to apply.
    #[tokio::test]
    async fn set_report_rejects_report_id_mismatched_against_command_header() {
        let mut bus = timeout_bus();
        let mut device = recording_device();
        let command = [
            0x23,                       // command low byte: report type Output (0x2), inline report ID 3
            Opcode::SetReport as u8,    // command high byte: SetReport opcode
            HidI2cRegister::Data as u8, // data register address, low byte (0x06)
            0x00,                       // data register address, high byte -> 0x0006
            0x04,                       // wLength low byte: 2 (self) + 1 (report ID) + 1 (payload)
            0x00,                       // wLength high byte
            0x04,                       // report ID in the payload disagrees with the header's 3
            0x5a,                       // report payload
        ];

        let result =
            Runner::<NoopBus, RecordingPin, RecordingHidDevice>::process_command(&command, &mut bus, &mut device).await;

        assert!(matches!(result, Err(Error::Protocol(ProtocolError::InvalidData))));
        assert_eq!(device.report_id, None);
    }

    #[tokio::test]
    async fn set_report_rejects_length_smaller_than_header() {
        let mut bus = timeout_bus();
        let mut device = recording_device();
        let command = [
            0x23,                       // command low byte: report type Output (0x2), inline report ID 3
            Opcode::SetReport as u8,    // command high byte: SetReport opcode
            HidI2cRegister::Data as u8, // data register address, low byte (0x06)
            0x00,                       // data register address, high byte -> 0x0006
            0x01,                       // length field, low byte
            0x00,                       // length field, high byte -> 1, too small to hold the header -> InvalidSize
        ];

        let result =
            Runner::<NoopBus, RecordingPin, RecordingHidDevice>::process_command(&command, &mut bus, &mut device).await;

        assert!(matches!(result, Err(Error::Protocol(ProtocolError::InvalidSize))));
    }

    #[tokio::test]
    async fn set_report_rejects_mismatched_wire_report_id() {
        let mut bus = timeout_bus();
        let mut device = recording_device();
        let command = [
            0x23,                       // command low byte: report type Output (0x2), inline report ID 3
            Opcode::SetReport as u8,    // command high byte: SetReport opcode
            HidI2cRegister::Data as u8, // data register address, low byte (0x06)
            0x00,                       // data register address, high byte -> 0x0006
            0x04,                       // length field, low byte
            0x00,                       // length field, high byte -> 4 total bytes
            0x04,                       // report ID in data payload = 4, mismatches header's 3 -> InvalidData
            0x5a,                       // report payload
        ];

        let result =
            Runner::<NoopBus, RecordingPin, RecordingHidDevice>::process_command(&command, &mut bus, &mut device).await;

        assert!(matches!(result, Err(Error::Protocol(ProtocolError::InvalidData))));
    }

    #[tokio::test]
    async fn get_report_rejects_output_report_type() {
        let mut bus = timeout_bus();
        let mut device = recording_device();
        let command = [
            0x21,                       // command low byte: report type Output (0x2), report ID 1
            Opcode::GetReport as u8, // command high byte: GetReport opcode (Output reports can't be read -> InvalidReportType)
            HidI2cRegister::Data as u8, // data register address, low byte (0x06)
            0x00,                    // data register address, high byte -> 0x0006
        ];

        let result =
            Runner::<NoopBus, RecordingPin, RecordingHidDevice>::process_command(&command, &mut bus, &mut device).await;

        assert!(matches!(result, Err(Error::Protocol(ProtocolError::InvalidReportType))));
    }

    #[tokio::test]
    async fn get_report_waits_for_the_host_read_before_answering() {
        let mut bus = scripted_timeout_bus(ScriptedBus {
            // No queued `listen` event, so `listen_for_response` times out.
            read_statuses: VecDeque::from([ReadStatus::Complete(3), ReadStatus::Complete(1)]),
            ..Default::default()
        });
        let mut device = recording_device();
        let command = [
            0x3f,                       // command low byte: report type Feature (0x3), report ID nibble 0xF = extended
            Opcode::GetReport as u8,    // command high byte: GetReport opcode
            0x21,                       // extended report ID (0x21)
            HidI2cRegister::Data as u8, // data register address, low byte (0x06)
            0x00,                       // data register address, high byte -> 0x0006
        ];

        let result =
            Runner::<ScriptedBus, RecordingPin, RecordingHidDevice>::process_command(&command, &mut bus, &mut device)
                .await;

        assert!(matches!(result, Err(Error::Protocol(ProtocolError::Timeout))));
        assert!(bus.bus.outgoing_reads.is_empty());
        // NOTE: unlike `read` and `write_unterminated`, `listen_for_response` propagates its
        // timeout without recovering the bus. Asserted here so the asymmetry is at least
        // visible; whether it should recover is a separate question.
        assert_eq!(bus.bus.recover_count, 0);
    }

    #[tokio::test]
    async fn reset_asserts_interrupt_and_first_read_acknowledges_completion() {
        let bus = ScriptedBus {
            read_statuses: VecDeque::from([ReadStatus::Complete(2)]),
            ..Default::default()
        };
        let mut resources = Resources::default();
        let (_service, mut runner) = Service::new(
            &mut resources,
            bus,
            RecordingPin::new(),
            recording_device(),
            hardware_version_info(),
            TimeoutSettings {
                device_response_timeout: Duration::from_millis(20),
                data_read_timeout: Duration::from_millis(20),
            },
        )
        .await
        .unwrap();

        runner.reset().await;

        assert!(runner.pending_reset);
        assert!(runner.attn_pin.asserted());
        assert_eq!(runner.hid_device.reset_count, 1);
        assert_eq!(runner.hid_device.power_state, Some(HidDevicePowerState::On));
        // The pin is driven low to assert; checking the recorded level rather than only the
        // handler's own bookkeeping is what makes this test able to fail if the GPIO is untouched.
        assert_eq!(runner.attn_pin.pin().level(), Some(PinLevel::Low));

        runner.reply_with_input_report().await.unwrap();

        assert!(!runner.pending_reset);
        assert!(!runner.attn_pin.asserted());
        assert_eq!(runner.attn_pin.pin().level(), Some(PinLevel::High));
        assert_eq!(
            runner.bus.bus.outgoing_reads.first().map(Vec::as_slice),
            Some(&[0x00, 0x00][..])
        );
    }

    #[tokio::test]
    async fn timeout_bus_reads_host_payload() {
        let mut bus = scripted_timeout_bus(ScriptedBus {
            incoming_writes: VecDeque::from([IncomingWrite {
                data: vec![0x10, 0x20, 0x30],
                status: WriteStatus::Stopped(3),
            }]),
            ..Default::default()
        });
        let mut buffer = [0; 4];

        let payload = bus.read(&mut buffer).await.unwrap();

        assert_eq!(payload, &[0x10, 0x20, 0x30]);
        assert_eq!(bus.bus.recover_count, 0);
    }

    #[tokio::test]
    async fn timeout_bus_drains_oversized_host_write() {
        let mut bus = scripted_timeout_bus(ScriptedBus {
            incoming_writes: VecDeque::from([
                IncomingWrite {
                    data: vec![0x10, 0x20],
                    status: WriteStatus::BufferFull(2),
                },
                IncomingWrite {
                    data: vec![0x30, 0x40],
                    status: WriteStatus::Stopped(2),
                },
            ]),
            ..Default::default()
        });
        let mut buffer = [0; 2];

        let result = bus.read(&mut buffer).await;

        assert!(matches!(result, Err(Error::Protocol(ProtocolError::InvalidData))));
        assert!(bus.bus.incoming_writes.is_empty());
        assert_eq!(bus.bus.recover_count, 0);
    }

    #[tokio::test]
    async fn timeout_bus_uses_zeroes_when_host_reads_past_response() {
        let mut bus = scripted_timeout_bus(ScriptedBus {
            read_statuses: VecDeque::from([ReadStatus::NeedMore(2), ReadStatus::Complete(3)]),
            ..Default::default()
        });

        bus.write(&[0xaa, 0xbb]).await.unwrap();

        assert_eq!(bus.bus.outgoing_reads.len(), 2);
        assert_eq!(
            bus.bus.outgoing_reads.first().map(Vec::as_slice),
            Some(&[0xaa, 0xbb][..])
        );
        assert_eq!(bus.bus.outgoing_reads.get(1).map(Vec::as_slice), Some(&[0; 8][..]));
        assert_eq!(bus.bus.recover_count, 0);
    }

    #[tokio::test]
    async fn timeout_bus_recovers_after_host_write_timeout() {
        let mut bus = scripted_timeout_bus(ScriptedBus::default());
        let mut buffer = [0; 4];

        let result = bus.read(&mut buffer).await;

        assert!(matches!(result, Err(Error::Protocol(ProtocolError::Timeout))));
        assert_eq!(bus.bus.recover_count, 1);
    }

    #[tokio::test]
    async fn timeout_bus_recovers_after_host_read_timeout() {
        let mut bus = scripted_timeout_bus(ScriptedBus::default());

        let result = bus.write(&[0xaa]).await;

        assert!(matches!(result, Err(Error::Protocol(ProtocolError::Timeout))));
        assert_eq!(bus.bus.recover_count, 1);
    }

    // ---------------------------------------------------------------------------------------
    // Spec-conformance tests.
    //
    // Reference for the wire format is the Linux `i2c-hid` host driver, which is the
    // authoritative consumer of what we emit:
    //
    //   i2c_hid_format_report()  - drivers/hid/i2c-hid/i2c-hid-core.c
    //       size_t length = sizeof(__le16);        /* reserve space to store size */
    //       if (report_id) buf[length++] = report_id;
    //       memcpy(buf + length, data, size); length += size;
    //       put_unaligned_le16(length, buf);
    //
    // i.e. a report on the wire is `[wLength(2)][report_id?][payload]`, and `wLength` counts
    // itself, the report ID (when the descriptor uses explicit report IDs), and the payload.
    // ---------------------------------------------------------------------------------------

    /// Host address used by the scripted bus; the service ignores it (single-address assumption).
    const HOST_ADDR: u8 = 0x2c;

    /// A SET_REPORT frame built exactly the way `i2c_hid_format_report` builds it must be accepted.
    #[tokio::test]
    async fn set_report_accepts_spec_conformant_frame_with_explicit_report_id() {
        let mut bus = timeout_bus();
        let mut device = recording_device();
        let command = [
            0x23,                       // command low byte: report type Output (0x2), inline report ID 3
            Opcode::SetReport as u8,    // command high byte: SetReport opcode
            HidI2cRegister::Data as u8, // data register address, low byte (0x06)
            0x00,                       // data register address, high byte -> 0x0006
            0x06,                       // wLength low byte: 2 (self) + 1 (report ID) + 3 (payload)
            0x00,                       // wLength high byte
            0x03,                       // report ID, repeated in the data payload per the spec
            0xaa,                       // report payload
            0xbb,
            0xcc,
        ];

        Runner::<NoopBus, RecordingPin, RecordingHidDevice>::process_command(&command, &mut bus, &mut device)
            .await
            .unwrap();

        assert_eq!(device.report_id, Some(ReportId(3)));
        assert_eq!(
            device.report_data.get(..device.report_len),
            Some(&[0xaa, 0xbb, 0xcc][..])
        );
        assert!(!device.feature_report);
    }

    /// The GET_REPORT response must carry the report ID, and `wLength` must count it.
    /// The host enforces this: `i2c_hid_get_report` rejects the response with `-EINVAL` when
    /// the first byte after the length header is not the requested report ID.
    #[tokio::test]
    async fn get_report_response_carries_report_id_for_explicit_descriptors() {
        let mut bus = scripted_timeout_bus(ScriptedBus {
            listen_requests: VecDeque::from([Request::Read(HOST_ADDR)]),
            read_statuses: VecDeque::from([ReadStatus::Complete(3), ReadStatus::Complete(1)]),
            ..Default::default()
        });
        let mut device = recording_device();
        let command = [
            0x3f,                       // command low byte: report type Feature (0x3), extended report ID
            Opcode::GetReport as u8,    // command high byte: GetReport opcode
            0x21,                       // extended report ID (0x21)
            HidI2cRegister::Data as u8, // data register address, low byte (0x06)
            0x00,                       // data register address, high byte -> 0x0006
        ];

        Runner::<ScriptedBus, RecordingPin, RecordingHidDevice>::process_command(&command, &mut bus, &mut device)
            .await
            .unwrap();

        // wLength = 2 (self) + 1 (report ID) + 1 (payload) = 4, then the report ID, then the payload.
        assert_eq!(
            bus.bus.outgoing_reads.first().map(Vec::as_slice),
            Some(&[0x04, 0x00, 0x21][..])
        );
        assert_eq!(bus.bus.outgoing_reads.get(1).map(Vec::as_slice), Some(&[0x5a][..]));
    }

    /// The mirror image: a single-collection device's GET_REPORT response carries no report ID.
    #[tokio::test]
    async fn get_report_response_omits_report_id_for_implicit_descriptors() {
        let mut bus = scripted_timeout_bus(ScriptedBus {
            listen_requests: VecDeque::from([Request::Read(HOST_ADDR)]),
            read_statuses: VecDeque::from([ReadStatus::Complete(2), ReadStatus::Complete(3)]),
            ..Default::default()
        });
        let mut device = crate::test_support::implicit_id_device().with_get_report_payload(&[0x11, 0x22, 0x33]);
        let command = [
            0x30,                       // command low byte: report type Feature (0x3), report ID 0
            Opcode::GetReport as u8,    // command high byte: GetReport opcode
            HidI2cRegister::Data as u8, // data register address, low byte (0x06)
            0x00,                       // data register address, high byte -> 0x0006
        ];

        Runner::<ScriptedBus, RecordingPin, RecordingHidDevice>::process_command(&command, &mut bus, &mut device)
            .await
            .unwrap();

        // wLength = 2 (self) + 3 (payload) = 5, with no report ID byte.
        assert_eq!(
            bus.bus.outgoing_reads.first().map(Vec::as_slice),
            Some(&[0x05, 0x00][..])
        );
        assert_eq!(
            bus.bus.outgoing_reads.get(1).map(Vec::as_slice),
            Some(&[0x11, 0x22, 0x33][..])
        );
    }

    /// The receive buffer must hold the largest SET_REPORT a host can legally send in one
    /// transaction. Per `i2c_hid_alloc_buffers` that worst case is 2 command register address
    /// bytes, 1 report type/ID byte, 1 opcode byte, 1 extended report ID byte, 2 data register
    /// address bytes, 2 wLength bytes, 1 report ID byte and N payload bytes - which is
    /// `N + 10`, not `N + 9`.
    #[test]
    fn write_buffer_holds_worst_case_set_report() {
        use typenum::Unsigned;

        const FRAMING_OVERHEAD: usize = 10;
        let payload_max = <RecordingHidDevice as crate::ConstrainedHidDevice>::MaxOutputOrFeatureSize::USIZE;

        assert_eq!(
            <RecordingHidDevice as crate::ConstrainedHidDevice>::WriteBufferSize::USIZE,
            payload_max + FRAMING_OVERHEAD
        );
    }

    // ---------------------------------------------------------------------------------------
    // Unsolicited input reports.
    //
    // None of this was reachable while the device mock reported `has_pending_input_report() ==
    // false` unconditionally.
    // ---------------------------------------------------------------------------------------

    async fn runner_with<'hw>(
        resources: &'hw mut Resources<ScriptedBus, RecordingPin, RecordingHidDevice>,
        bus: ScriptedBus,
        device: RecordingHidDevice,
    ) -> Runner<'hw, ScriptedBus, RecordingPin, RecordingHidDevice> {
        let (_service, runner) = Service::new(
            resources,
            bus,
            RecordingPin::new(),
            device,
            hardware_version_info(),
            TimeoutSettings {
                device_response_timeout: Duration::from_millis(20),
                data_read_timeout: Duration::from_millis(20),
            },
        )
        .await
        .unwrap();
        runner
    }

    /// Section 6.1.2: an input report from a device with explicit report IDs is framed as
    /// `[length(2)][report ID][report]`, with the length counting all three.
    #[tokio::test]
    async fn input_report_is_framed_with_report_id_for_explicit_descriptors() {
        let bus = ScriptedBus {
            read_statuses: VecDeque::from([ReadStatus::Complete(3), ReadStatus::Complete(2)]),
            ..Default::default()
        };
        let device = recording_device().with_pending_input(ReportId(3), &[0xde, 0xad]);
        let mut resources = Resources::default();
        let mut runner = runner_with(&mut resources, bus, device).await;

        runner.reply_with_input_report().await.unwrap();

        // wLength = 2 (self) + 1 (report ID) + 2 (payload) = 5
        assert_eq!(
            runner.bus.bus.outgoing_reads.first().map(Vec::as_slice),
            Some(&[0x05, 0x00, 0x03][..])
        );
        assert_eq!(
            runner.bus.bus.outgoing_reads.get(1).map(Vec::as_slice),
            Some(&[0xde, 0xad][..])
        );
        // Report consumed, so the interrupt is released.
        assert!(!runner.attn_pin.asserted());
    }

    /// The same report from a single-collection device carries no report ID byte.
    #[tokio::test]
    async fn input_report_is_framed_without_report_id_for_implicit_descriptors() {
        let bus = ScriptedBus {
            read_statuses: VecDeque::from([ReadStatus::Complete(2), ReadStatus::Complete(2)]),
            ..Default::default()
        };
        let device = crate::test_support::implicit_id_device().with_pending_input(ReportId(0), &[0xde, 0xad]);
        let mut resources = Resources::default();
        let mut runner = runner_with(&mut resources, bus, device).await;

        runner.reply_with_input_report().await.unwrap();

        // wLength = 2 (self) + 2 (payload) = 4
        assert_eq!(
            runner.bus.bus.outgoing_reads.first().map(Vec::as_slice),
            Some(&[0x04, 0x00][..])
        );
        assert_eq!(
            runner.bus.bus.outgoing_reads.get(1).map(Vec::as_slice),
            Some(&[0xde, 0xad][..])
        );
    }

    /// Section 7.2.1: the host polling with nothing queued gets a zero-length report rather than
    /// a stalled bus.
    #[tokio::test]
    async fn host_polling_with_no_pending_report_gets_a_zero_length_report() {
        let bus = ScriptedBus {
            read_statuses: VecDeque::from([ReadStatus::Complete(2)]),
            ..Default::default()
        };
        let mut resources = Resources::default();
        let mut runner = runner_with(&mut resources, bus, recording_device()).await;

        runner.reply_with_input_report().await.unwrap();

        assert_eq!(
            runner.bus.bus.outgoing_reads.first().map(Vec::as_slice),
            Some(&[0x00, 0x00][..])
        );
    }

    // ---------------------------------------------------------------------------------------
    // Failure paths.
    //
    // All of these were unreachable while the mocks used `Infallible`.
    // ---------------------------------------------------------------------------------------

    #[tokio::test]
    async fn bus_error_while_reading_the_host_write_is_reported() {
        let mut bus = scripted_timeout_bus(ScriptedBus {
            fail_next_write: Some(ErrorKind::Overrun),
            ..Default::default()
        });
        let mut buffer = [0; 8];

        let result = bus.read(&mut buffer).await;

        assert!(matches!(result, Err(Error::Bus(ErrorKind::Overrun))));
    }

    #[tokio::test]
    async fn bus_error_while_answering_the_host_read_is_reported() {
        let mut bus = scripted_timeout_bus(ScriptedBus {
            fail_next_read: Some(ErrorKind::ArbitrationLoss),
            ..Default::default()
        });

        let result = bus.write(&[0xaa]).await;

        assert!(matches!(result, Err(Error::Bus(ErrorKind::ArbitrationLoss))));
    }

    /// A bus that cannot even be recovered after a timeout reports the recovery failure, not the
    /// timeout that triggered it.
    #[tokio::test]
    async fn failure_to_recover_after_timeout_is_reported() {
        let mut bus = scripted_timeout_bus(ScriptedBus {
            fail_recover: Some(ErrorKind::Bus),
            ..Default::default()
        });
        let mut buffer = [0; 8];

        let result = bus.read(&mut buffer).await;

        assert!(matches!(result, Err(Error::Bus(ErrorKind::Bus))));
        assert_eq!(bus.bus.recover_count, 1);
    }

    #[tokio::test]
    async fn device_failure_during_set_report_is_reported() {
        let mut bus = timeout_bus();
        let mut device = recording_device();
        device.fail_set_report = Some(HidError::TriggerReset);
        let command = [
            0x23,
            Opcode::SetReport as u8,
            HidI2cRegister::Data as u8,
            0x00,
            0x04,
            0x00,
            0x03,
            0x5a,
        ];

        let result =
            Runner::<NoopBus, RecordingPin, RecordingHidDevice>::process_command(&command, &mut bus, &mut device).await;

        assert!(matches!(result, Err(Error::Device(HidError::TriggerReset))));
    }

    #[tokio::test]
    async fn device_failure_during_set_power_is_reported() {
        let mut bus = timeout_bus();
        let mut device = recording_device();
        device.fail_set_power = Some(HidError::TriggerReset);

        let result = Runner::<NoopBus, RecordingPin, RecordingHidDevice>::process_command(
            &[0x01, Opcode::SetPower as u8],
            &mut bus,
            &mut device,
        )
        .await;

        assert!(matches!(result, Err(Error::Device(HidError::TriggerReset))));
        assert_eq!(device.power_state, None);
    }

    #[tokio::test]
    async fn device_failure_during_get_report_is_reported() {
        let mut bus = scripted_timeout_bus(ScriptedBus {
            listen_requests: VecDeque::from([Request::Read(HOST_ADDR)]),
            ..Default::default()
        });
        let mut device = recording_device();
        device.fail_get_report = Some(HidError::TriggerReset);
        let command = [0x31, Opcode::GetReport as u8, HidI2cRegister::Data as u8, 0x00];

        let result =
            Runner::<ScriptedBus, RecordingPin, RecordingHidDevice>::process_command(&command, &mut bus, &mut device)
                .await;

        assert!(matches!(result, Err(Error::Device(HidError::TriggerReset))));
    }

    /// A GPIO that refuses to move must not take the reset path down with it: the service logs
    /// and carries on, because a device-initiated reset is the only recovery it has.
    #[tokio::test]
    async fn reset_survives_an_attn_pin_that_cannot_be_driven() {
        let mut resources = Resources::default();
        let (_service, mut runner) = Service::new(
            &mut resources,
            ScriptedBus::default(),
            // One successful transition for `AttnPinHandler::new`, then failure.
            RecordingPin::failing_after(1),
            recording_device(),
            hardware_version_info(),
            TimeoutSettings::default(),
        )
        .await
        .unwrap();

        runner.reset().await;

        assert_eq!(runner.hid_device.reset_count, 1);
        // The assert failed, so the handler never recorded the interrupt as raised.
        assert!(!runner.attn_pin.asserted());
        assert!(runner.pending_reset);
    }

    /// An oversized report cannot be framed in the 16-bit length field, so it is rejected
    /// instead of being silently truncated by an `as u16` cast.
    #[test]
    fn report_header_rejects_a_report_too_large_for_the_length_field() {
        use crate::wire::{ReportFraming, ReportHeader};

        assert_eq!(
            ReportHeader::new(usize::from(u16::MAX), ReportId(1), ReportFraming::Explicit),
            Err(ProtocolError::InvalidSize)
        );
    }
}
