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
            //
            // `WriteStatus` is `#[non_exhaustive]`, so this fallback is required for forward
            // compatibility. The crate currently exposes only `Stopped`, `Restarted`, and
            // `BufferFull`, all handled above; no current value can reach this arm, so it is
            // intentionally not covered by tests.
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

    /// Upper bound on how many steps a single [`Transaction`] may queue. A script longer than this
    /// is a sign the test is describing a whole session rather than one transaction.
    const MAX_SCRIPT_STEPS: usize = 16;

    #[derive(Debug)]
    struct IncomingWrite {
        data: Vec<u8>,
        status: WriteStatus,
    }

    /// One scripted answer for `listen()`. Position in the queue is meaningful: the Nth `listen()`
    /// consumes the Nth step.
    #[derive(Debug)]
    enum ListenStep {
        Success(Request),
        Error(MockBusError),
        /// Pop the step, then never complete, so the caller's timeout fires. Steps queued behind
        /// this one survive the cancellation.
        Pending,
    }

    /// One scripted answer for `respond_to_write()`.
    #[derive(Debug)]
    enum RespondToWriteStep {
        Success(IncomingWrite),
        Error(MockBusError),
        Pending,
    }

    /// The bytes a `respond_to_read()` step expects to be offered, and the status it answers with.
    #[derive(Debug)]
    struct ExpectedRead {
        expected: Vec<u8>,
        status: ReadStatus,
        /// False for steps migrated from tests that never asserted the offered bytes: the offer is
        /// still recorded into `outgoing_reads`, but not compared.
        assert_contents: bool,
    }

    impl ExpectedRead {
        fn new(expected: &[u8], status: ReadStatus) -> Self {
            Self {
                expected: expected.to_vec(),
                status,
                assert_contents: true,
            }
        }

        /// Records the offered bytes without asserting on them. Use only where the test under
        /// migration made no claim about what was offered.
        fn unchecked(status: ReadStatus) -> Self {
            Self {
                expected: Vec::new(),
                status,
                assert_contents: false,
            }
        }
    }

    /// One scripted answer for `respond_to_read()`.
    #[derive(Debug)]
    enum RespondToReadStep {
        Success(ExpectedRead),
        Error(MockBusError),
        Pending,
    }

    /// Selects which per-operation step queue a builder call targets.
    #[derive(Debug, Clone, Copy)]
    enum ScriptOp {
        Listen,
        RespondToWrite,
        RespondToRead,
    }

    /// A bus driven by three independent, ordered step queues - one per transaction primitive.
    ///
    /// Each call pops exactly one step from its own queue, so "succeed, succeed, then fail" and
    /// "succeed, then hang" are both expressible. An empty queue stays pending forever, which is
    /// what the timeout tests rely on.
    #[derive(Default)]
    struct ScriptedBus {
        listen_steps: VecDeque<ListenStep>,
        respond_to_write_steps: VecDeque<RespondToWriteStep>,
        respond_to_read_steps: VecDeque<RespondToReadStep>,
        outgoing_reads: Vec<Vec<u8>>,
        recover_count: usize,
        /// When set, `recover()` fails with this error. Recovery is not one of the three
        /// transaction queues, so it keeps its latching behaviour.
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
            let Some(step) = self.listen_steps.pop_front() else {
                return core::future::pending().await;
            };
            match step {
                ListenStep::Pending => core::future::pending().await,
                ListenStep::Error(error) => Err(error),
                ListenStep::Success(request) => Ok(request),
            }
        }

        async fn respond_to_read(&mut self, buf: &[u8]) -> Result<ReadStatus, Self::Error> {
            let Some(step) = self.respond_to_read_steps.pop_front() else {
                return core::future::pending().await;
            };
            match step {
                RespondToReadStep::Pending => core::future::pending().await,
                RespondToReadStep::Error(error) => Err(error),
                RespondToReadStep::Success(read) => {
                    self.outgoing_reads.push(buf.to_vec());
                    if read.assert_contents {
                        assert_eq!(
                            buf,
                            read.expected.as_slice(),
                            "respond_to_read was offered bytes the script did not expect"
                        );
                    }
                    Ok(read.status)
                }
            }
        }

        async fn respond_to_write(&mut self, buf: &mut [u8]) -> Result<WriteStatus, Self::Error> {
            let Some(step) = self.respond_to_write_steps.pop_front() else {
                return core::future::pending().await;
            };
            match step {
                RespondToWriteStep::Pending => core::future::pending().await,
                RespondToWriteStep::Error(error) => Err(error),
                RespondToWriteStep::Success(write) => {
                    for (destination, source) in buf.iter_mut().zip(write.data.iter()) {
                        *destination = *source;
                    }
                    Ok(write.status)
                }
            }
        }
    }

    /// Number of bytes a write status claims to have transferred.
    fn write_status_count(status: WriteStatus) -> usize {
        match status {
            WriteStatus::Stopped(bytes) | WriteStatus::Restarted(bytes) | WriteStatus::BufferFull(bytes) => bytes,
            _ => 0,
        }
    }

    /// Rebuilds `status` with a new byte count, preserving the variant.
    fn write_status_with_count(status: WriteStatus, count: usize) -> WriteStatus {
        match status {
            WriteStatus::Stopped(_) => WriteStatus::Stopped(count),
            WriteStatus::Restarted(_) => WriteStatus::Restarted(count),
            WriteStatus::BufferFull(_) => WriteStatus::BufferFull(count),
            other => other,
        }
    }

    /// Builds the script for one host transaction, including any follow-on sub-transactions.
    ///
    /// The builder validates at construction time: statuses must agree with the data they
    /// describe, `then_read` must carry at least one chunk, appends are only legal while the
    /// initial write is still open, and the total step count is capped at [`MAX_SCRIPT_STEPS`].
    struct Transaction {
        initial_request: Request,
        bus: ScriptedBus,
        scripted_steps: usize,
        /// True while `append_to_initial_write` may still extend the initial `IncomingWrite`.
        initial_write_open: bool,
        /// True while the initial `Request::Read` has no `respond_to_read` script yet, so the
        /// first `then_read` must not queue a duplicate listen step for it.
        initial_read_response_unscripted: bool,
    }

    impl Transaction {
        /// A transaction the host opens by writing `data` to us.
        fn write(data: &[u8], status: WriteStatus) -> Self {
            assert!(
                write_status_count(status) <= data.len(),
                "write status claims more bytes than the script provides"
            );
            let mut transaction = Self {
                initial_request: Request::Write(HOST_ADDR),
                bus: ScriptedBus::default(),
                scripted_steps: 0,
                initial_write_open: true,
                initial_read_response_unscripted: false,
            };
            transaction.reserve_steps(1);
            transaction
                .bus
                .respond_to_write_steps
                .push_back(RespondToWriteStep::Success(IncomingWrite {
                    data: data.to_vec(),
                    status,
                }));
            transaction
        }

        /// A transaction the host opens by reading from us. No listen step is queued: the initial
        /// request is returned by `finish()` and fed to the service directly.
        fn read() -> Self {
            Self {
                initial_request: Request::Read(HOST_ADDR),
                bus: ScriptedBus::default(),
                scripted_steps: 0,
                initial_write_open: false,
                initial_read_response_unscripted: true,
            }
        }

        /// Rejects scripts longer than [`MAX_SCRIPT_STEPS`], counted cumulatively.
        fn reserve_steps(&mut self, additional: usize) {
            let total = self.scripted_steps.checked_add(additional);
            assert!(
                !total.is_none_or(|total| total > MAX_SCRIPT_STEPS),
                "transaction script exceeds {MAX_SCRIPT_STEPS} steps"
            );
            self.scripted_steps = total.unwrap_or(MAX_SCRIPT_STEPS);
        }

        /// Extends the initial host write with more bytes, keeping its original status variant and
        /// growing its count. Consumes no step. Legal only before any other builder call.
        fn append_to_initial_write(mut self, data: &[u8]) -> Self {
            assert!(
                self.initial_write_open,
                "append_to_initial_write is only legal immediately after Transaction::write"
            );
            let mut appended = false;
            if let Some(RespondToWriteStep::Success(write)) = self.bus.respond_to_write_steps.front_mut() {
                write.data.extend_from_slice(data);
                write.status = write_status_with_count(write.status, write_status_count(write.status) + data.len());
                appended = true;
            }
            assert!(appended, "the initial write step is always a success step");
            self
        }

        /// The host restarts into another write to us.
        fn then_write(mut self, data: &[u8], status: WriteStatus) -> Self {
            assert!(
                write_status_count(status) <= data.len(),
                "write status claims more bytes than the script provides"
            );
            self.close_initial();
            self.reserve_steps(2);
            self.bus
                .listen_steps
                .push_back(ListenStep::Success(Request::Write(HOST_ADDR)));
            self.bus
                .respond_to_write_steps
                .push_back(RespondToWriteStep::Success(IncomingWrite {
                    data: data.to_vec(),
                    status,
                }));
            self
        }

        /// The host reads from us, taking the response in `chunks`. Each chunk is one
        /// `respond_to_read` call: the bytes we must offer and the status the host answers with.
        fn then_read<const N: usize>(mut self, chunks: [(&[u8], ReadStatus); N]) -> Self {
            assert!(N >= 1, "then_read requires at least one chunk");
            let needs_listen = !self.initial_read_response_unscripted;
            self.close_initial();
            self.reserve_steps(if needs_listen { N + 1 } else { N });
            if needs_listen {
                self.bus
                    .listen_steps
                    .push_back(ListenStep::Success(Request::Read(HOST_ADDR)));
            }
            for (expected, status) in chunks {
                assert_read_status(status, expected.len());
                self.bus
                    .respond_to_read_steps
                    .push_back(RespondToReadStep::Success(ExpectedRead::new(expected, status)));
            }
            self
        }

        /// Queues a bare listen step - for `Stop`, `RepeatedStart`, or a deliberately malformed
        /// direction sequence.
        fn then_request(mut self, request: Request) -> Self {
            self.close_initial();
            self.reserve_steps(1);
            self.bus.listen_steps.push_back(ListenStep::Success(request));
            self
        }

        /// Fails the next call to `operation`, at the current position in that operation's queue.
        fn fail_next(mut self, operation: ScriptOp, error: MockBusError) -> Self {
            self.initial_write_open = false;
            self.reserve_steps(1);
            match operation {
                ScriptOp::Listen => self.bus.listen_steps.push_back(ListenStep::Error(error)),
                ScriptOp::RespondToWrite => self
                    .bus
                    .respond_to_write_steps
                    .push_back(RespondToWriteStep::Error(error)),
                ScriptOp::RespondToRead => self
                    .bus
                    .respond_to_read_steps
                    .push_back(RespondToReadStep::Error(error)),
            }
            self
        }

        /// Hangs the next call to `operation` so the caller's timeout fires. Later steps in that
        /// queue survive the cancellation.
        fn timeout_next(mut self, operation: ScriptOp) -> Self {
            self.initial_write_open = false;
            self.reserve_steps(1);
            match operation {
                ScriptOp::Listen => self.bus.listen_steps.push_back(ListenStep::Pending),
                ScriptOp::RespondToWrite => self.bus.respond_to_write_steps.push_back(RespondToWriteStep::Pending),
                ScriptOp::RespondToRead => self.bus.respond_to_read_steps.push_back(RespondToReadStep::Pending),
            }
            self
        }

        /// Makes bus recovery fail. Consumes no step.
        fn recover_fails_with(mut self, error: MockBusError) -> Self {
            self.initial_write_open = false;
            self.bus.fail_recover = Some(error);
            self
        }

        fn finish(self) -> (Request, ScriptedBus) {
            (self.initial_request, self.bus)
        }

        fn close_initial(&mut self) {
            self.initial_write_open = false;
            self.initial_read_response_unscripted = false;
        }
    }

    /// Builder-time check that a read status agrees with the chunk it describes.
    fn assert_read_status(status: ReadStatus, len: usize) {
        match status {
            ReadStatus::Complete(bytes) | ReadStatus::NeedMore(bytes) => assert_eq!(
                bytes, len,
                "Complete/NeedMore must report the full length of the offered chunk"
            ),
            ReadStatus::EarlyStop(bytes) => assert!(
                bytes <= len,
                "EarlyStop cannot report more bytes than the offered chunk holds"
            ),
            _ => {}
        }
    }

    /// Compares recorded host reads without indexing (`clippy::indexing_slicing` is denied).
    fn assert_outgoing_reads(actual: &[Vec<u8>], expected: &[&[u8]]) {
        assert_eq!(actual.len(), expected.len(), "unexpected number of host reads");
        for (index, (actual, expected)) in actual.iter().zip(expected.iter()).enumerate() {
            assert_eq!(actual.as_slice(), *expected, "host read {index} differs");
        }
    }

    /// Every scripted step must have been consumed; leftovers mean the test under-drove the bus.
    fn assert_script_consumed(bus: &ScriptedBus) {
        assert!(bus.listen_steps.is_empty(), "unconsumed listen steps remain");
        assert!(
            bus.respond_to_write_steps.is_empty(),
            "unconsumed respond_to_write steps remain"
        );
        assert!(
            bus.respond_to_read_steps.is_empty(),
            "unconsumed respond_to_read steps remain"
        );
    }

    /// Compares recorded GPIO transitions without indexing.
    fn assert_pin_levels(actual: &[PinLevel], expected: &[PinLevel]) {
        assert_eq!(actual.len(), expected.len(), "unexpected number of pin transitions");
        for (index, (actual, expected)) in actual.iter().zip(expected.iter()).enumerate() {
            assert_eq!(actual, expected, "pin transition {index} differs");
        }
    }

    /// Fails the test rather than hanging the suite if a future never settles. The bus deadline
    /// stays at 20 ms; this is the outer backstop.
    #[allow(clippy::expect_used)]
    async fn watchdog<F: core::future::Future>(future: F) -> F::Output {
        tokio::time::timeout(std::time::Duration::from_millis(500), future)
            .await
            .expect("test exceeded 500 ms watchdog")
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
            // No queued `listen` event, so `listen_for_response` times out. These read steps are
            // deliberately never reached; the original test made no claim about offered bytes.
            respond_to_read_steps: VecDeque::from([
                RespondToReadStep::Success(ExpectedRead::unchecked(ReadStatus::Complete(3))),
                RespondToReadStep::Success(ExpectedRead::unchecked(ReadStatus::Complete(1))),
            ]),
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
        // CHARACTERISATION, not a specification: `listen_for_response` propagates its
        // timeout WITHOUT attempting `recover()`, unlike `read` and `write_unterminated`.
        // This assertion pins the behaviour as it exists today so a change is noticed; whether
        // this asymmetry is correct is an open design question, not a settled requirement.
        assert_eq!(bus.bus.recover_count, 0);
    }

    #[tokio::test]
    async fn reset_asserts_interrupt_and_first_read_acknowledges_completion() {
        let bus = ScriptedBus {
            respond_to_read_steps: VecDeque::from([RespondToReadStep::Success(ExpectedRead::new(
                &[0x00, 0x00],
                ReadStatus::Complete(2),
            ))]),
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
        // The full transition history, not just the final level: deassert at construction,
        // assert on reset, deassert once the host has read the acknowledgement.
        assert_pin_levels(
            &runner.attn_pin.pin().levels,
            &[PinLevel::High, PinLevel::Low, PinLevel::High],
        );
        assert_eq!(
            runner.bus.bus.outgoing_reads.first().map(Vec::as_slice),
            Some(&[0x00, 0x00][..])
        );
    }

    #[tokio::test]
    async fn timeout_bus_reads_host_payload() {
        let mut bus = scripted_timeout_bus(ScriptedBus {
            respond_to_write_steps: VecDeque::from([RespondToWriteStep::Success(IncomingWrite {
                data: vec![0x10, 0x20, 0x30],
                status: WriteStatus::Stopped(3),
            })]),
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
            respond_to_write_steps: VecDeque::from([
                RespondToWriteStep::Success(IncomingWrite {
                    data: vec![0x10, 0x20],
                    status: WriteStatus::BufferFull(2),
                }),
                RespondToWriteStep::Success(IncomingWrite {
                    data: vec![0x30, 0x40],
                    status: WriteStatus::Stopped(2),
                }),
            ]),
            ..Default::default()
        });
        let mut buffer = [0; 2];

        let result = bus.read(&mut buffer).await;

        assert!(matches!(result, Err(Error::Protocol(ProtocolError::InvalidData))));
        assert!(bus.bus.respond_to_write_steps.is_empty());
        assert_eq!(bus.bus.recover_count, 0);
    }

    #[tokio::test]
    async fn timeout_bus_uses_zeroes_when_host_reads_past_response() {
        let mut bus = scripted_timeout_bus(ScriptedBus {
            respond_to_read_steps: VecDeque::from([
                RespondToReadStep::Success(ExpectedRead::new(&[0xaa, 0xbb], ReadStatus::NeedMore(2))),
                // The padding offer is the service's 8-byte zero buffer; the host takes 3 of it.
                RespondToReadStep::Success(ExpectedRead::new(&[0; 8], ReadStatus::Complete(3))),
            ]),
            ..Default::default()
        });

        bus.write(&[0xaa, 0xbb]).await.unwrap();

        assert_outgoing_reads(&bus.bus.outgoing_reads, &[&[0xaa, 0xbb], &[0; 8]]);
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
            listen_steps: VecDeque::from([ListenStep::Success(Request::Read(HOST_ADDR))]),
            respond_to_read_steps: VecDeque::from([
                RespondToReadStep::Success(ExpectedRead::new(&[0x04, 0x00, 0x21], ReadStatus::Complete(3))),
                RespondToReadStep::Success(ExpectedRead::new(&[0x5a], ReadStatus::Complete(1))),
            ]),
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

    /// The mirror image: a device with no Report ID items omits the report ID from its
    /// GET_REPORT response.
    #[tokio::test]
    async fn get_report_response_omits_report_id_for_implicit_descriptors() {
        let mut bus = scripted_timeout_bus(ScriptedBus {
            listen_steps: VecDeque::from([ListenStep::Success(Request::Read(HOST_ADDR))]),
            respond_to_read_steps: VecDeque::from([
                RespondToReadStep::Success(ExpectedRead::new(&[0x05, 0x00], ReadStatus::Complete(2))),
                RespondToReadStep::Success(ExpectedRead::new(&[0x11, 0x22, 0x33], ReadStatus::Complete(3))),
            ]),
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
            respond_to_read_steps: VecDeque::from([
                RespondToReadStep::Success(ExpectedRead::new(&[0x05, 0x00, 0x03], ReadStatus::Complete(3))),
                RespondToReadStep::Success(ExpectedRead::new(&[0xde, 0xad], ReadStatus::Complete(2))),
            ]),
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

    /// The same report from a device with no Report ID items carries no report ID byte.
    #[tokio::test]
    async fn input_report_is_framed_without_report_id_for_implicit_descriptors() {
        let bus = ScriptedBus {
            respond_to_read_steps: VecDeque::from([
                RespondToReadStep::Success(ExpectedRead::new(&[0x04, 0x00], ReadStatus::Complete(2))),
                RespondToReadStep::Success(ExpectedRead::new(&[0xde, 0xad], ReadStatus::Complete(2))),
            ]),
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
            respond_to_read_steps: VecDeque::from([RespondToReadStep::Success(ExpectedRead::new(
                &[0x00, 0x00],
                ReadStatus::Complete(2),
            ))]),
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
            respond_to_write_steps: VecDeque::from([RespondToWriteStep::Error(ErrorKind::Overrun)]),
            ..Default::default()
        });
        let mut buffer = [0; 8];

        let result = bus.read(&mut buffer).await;

        assert!(matches!(result, Err(Error::Bus(ErrorKind::Overrun))));
    }

    #[tokio::test]
    async fn bus_error_while_answering_the_host_read_is_reported() {
        let mut bus = scripted_timeout_bus(ScriptedBus {
            respond_to_read_steps: VecDeque::from([RespondToReadStep::Error(ErrorKind::ArbitrationLoss)]),
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
            listen_steps: VecDeque::from([ListenStep::Success(Request::Read(HOST_ADDR))]),
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

    // ---------------------------------------------------------------------------------------
    // Harness self-tests.
    //
    // The mock is now load-bearing: if its step queues stop being positional, or the builder
    // stops rejecting nonsense scripts, every test above silently gets weaker. These lock that
    // behaviour down.
    // ---------------------------------------------------------------------------------------

    /// The old `fail_next_read` flag failed whichever read came next, so "succeed, then fail" was
    /// inexpressible. Position in the queue now decides.
    #[tokio::test]
    async fn mid_transaction_error_is_positional() {
        let mut bus = scripted_timeout_bus(ScriptedBus {
            respond_to_read_steps: VecDeque::from([
                RespondToReadStep::Success(ExpectedRead::new(&[0xaa], ReadStatus::Complete(1))),
                RespondToReadStep::Error(ErrorKind::Bus),
            ]),
            ..Default::default()
        });

        watchdog(async {
            bus.write(&[0xaa]).await.unwrap();
            let result = bus.write(&[0xbb]).await;
            assert!(matches!(result, Err(Error::Bus(ErrorKind::Bus))));
        })
        .await;

        // The failing step recorded nothing, so only the first offer is visible.
        assert_outgoing_reads(&bus.bus.outgoing_reads, &[&[0xaa]]);
        assert_script_consumed(&bus.bus);
    }

    /// A `Pending` step hangs exactly one call; steps queued behind it survive the timeout
    /// cancelling that call's future.
    #[tokio::test]
    async fn mid_transaction_pending_is_positional() {
        let mut bus = scripted_timeout_bus(ScriptedBus {
            respond_to_read_steps: VecDeque::from([
                RespondToReadStep::Success(ExpectedRead::new(&[0xaa], ReadStatus::Complete(1))),
                RespondToReadStep::Pending,
                RespondToReadStep::Success(ExpectedRead::new(&[0xcc], ReadStatus::Complete(1))),
            ]),
            ..Default::default()
        });

        watchdog(bus.write(&[0xaa])).await.unwrap();
        let result = watchdog(bus.write(&[0xbb])).await;

        assert!(matches!(result, Err(Error::Protocol(ProtocolError::Timeout))));
        assert_eq!(bus.bus.recover_count, 1);
        // The step queued behind the pending one is still there.
        assert_eq!(bus.bus.respond_to_read_steps.len(), 1);

        watchdog(bus.write(&[0xcc])).await.unwrap();

        assert_outgoing_reads(&bus.bus.outgoing_reads, &[&[0xaa], &[0xcc]]);
        assert_script_consumed(&bus.bus);
    }

    #[test]
    #[should_panic(expected = "then_read requires at least one chunk")]
    fn builder_rejects_empty_then_read() {
        let _rejected = Transaction::write(&[0x01], WriteStatus::Stopped(1)).then_read::<0>([]);
    }

    #[test]
    #[should_panic(expected = "append_to_initial_write is only legal")]
    fn builder_rejects_append_after_other_step() {
        let _rejected = Transaction::write(&[0x01], WriteStatus::Restarted(1))
            .then_request(Request::Stop(HOST_ADDR))
            .append_to_initial_write(&[0x02]);
    }

    #[test]
    #[should_panic(expected = "exceeds 16 steps")]
    fn builder_enforces_cumulative_step_limit() {
        // 1 step for the initial write, then 2 per `then_read`: the eighth crosses the cap.
        let mut transaction = Transaction::write(&[0x01], WriteStatus::Restarted(1));
        for _ in 0..8 {
            transaction = transaction.then_read([(&[0xaa][..], ReadStatus::Complete(1))]);
        }
    }

    /// A transaction the host opened with a read already has its request; the first `then_read`
    /// scripts the response to *that* request rather than inventing a second one.
    #[test]
    fn initial_read_then_read_chunks_does_not_duplicate_request() {
        let (request, bus) = Transaction::read()
            .then_read([(&[0x00, 0x00][..], ReadStatus::Complete(2))])
            .finish();

        assert_eq!(request, Request::Read(HOST_ADDR));
        assert!(bus.listen_steps.is_empty());
        assert_eq!(bus.respond_to_read_steps.len(), 1);
    }

    /// Covers the rest of the builder surface, including that appends keep the original status
    /// variant and only grow its count.
    #[test]
    fn builder_scripts_each_operation_into_its_own_queue() {
        let (request, bus) = Transaction::write(&[0x04, 0x00], WriteStatus::Restarted(2))
            .append_to_initial_write(&[0x31])
            .then_read([
                (&[0x05, 0x00][..], ReadStatus::NeedMore(2)),
                (&[0x11][..], ReadStatus::Complete(1)),
            ])
            .then_write(&[0x06, 0x00], WriteStatus::Stopped(2))
            .then_request(Request::Stop(HOST_ADDR))
            .fail_next(ScriptOp::Listen, ErrorKind::Bus)
            .fail_next(ScriptOp::RespondToWrite, ErrorKind::Overrun)
            .fail_next(ScriptOp::RespondToRead, ErrorKind::ArbitrationLoss)
            .timeout_next(ScriptOp::Listen)
            .timeout_next(ScriptOp::RespondToWrite)
            .timeout_next(ScriptOp::RespondToRead)
            .recover_fails_with(ErrorKind::Bus)
            .finish();

        assert_eq!(request, Request::Write(HOST_ADDR));
        // then_read's listen, then_write's listen, the Stop, the failing listen, the hung listen.
        assert_eq!(bus.listen_steps.len(), 5);
        // Initial write, then_write's write, the failing write, the hung write.
        assert_eq!(bus.respond_to_write_steps.len(), 4);
        // Two chunks, the failing read, the hung read.
        assert_eq!(bus.respond_to_read_steps.len(), 4);
        assert_eq!(bus.fail_recover, Some(ErrorKind::Bus));

        let initial = bus.respond_to_write_steps.front();
        let Some(RespondToWriteStep::Success(initial)) = initial else {
            let described = format!("{initial:?}");
            assert_eq!(described, "the initial write step", "initial write step was replaced");
            return;
        };
        assert_eq!(initial.data, vec![0x04, 0x00, 0x31]);
        // Restarted stays Restarted; only the count grows by the appended length.
        assert_eq!(initial.status, WriteStatus::Restarted(3));
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

    // ---------------------------------------------------------------------------------------
    // Register-dispatch tests (pass A).
    //
    // Every test here drives `process_register_access`, i.e. the real entry point the run loop
    // uses for a host write. Expected bytes are derived from the framing rule in spec sections
    // 6.2.1/6.2.2 and 7.2.3.1: `[wLength(2, LE)][ReportID?][payload]`, where `wLength` counts
    // itself, the report ID and the payload. Parser internals live in `crate::wire`.
    // ---------------------------------------------------------------------------------------

    /// The `0b1111` extended-report-ID sentinel of spec sections 7.2.2.4/7.2.3.4 binds only the
    /// Command register's 4-bit report-ID *nibble*, and its "Third Byte" is appended to the
    /// command, never to the data payload. The Output register (section 6.2) has no nibble: its
    /// report ID is a full byte, so `0x0F` there is the literal report ID 15.
    #[tokio::test]
    async fn output_register_treats_0x0f_as_literal_report_id() {
        let (_request, bus) = Transaction::write(
            &[
                0x04, 0x00, // Output register address, little-endian
                0x04, 0x00, // wLength = 2 (self) + 1 (report ID) + 1 (payload)
                0x0f, // report ID 15 - a literal full byte, NOT the command-register sentinel
                0x5a, // report payload
            ],
            WriteStatus::Stopped(6),
        )
        .finish();
        let mut resources = Resources::default();
        let mut runner = runner_with(&mut resources, bus, recording_device()).await;

        watchdog(runner.process_register_access()).await.unwrap();

        assert_eq!(runner.hid_device.report_id, Some(ReportId(0x0f)));
        // If `0x0f` had been misread as a sentinel, `0x5a` would have been eaten as the real
        // report ID and the payload would be empty.
        assert_eq!(
            runner.hid_device.report_data.get(..runner.hid_device.report_len),
            Some(&[0x5a][..])
        );
        assert!(!runner.hid_device.feature_report);
        assert_eq!(runner.bus.bus.recover_count, 0);
        assert_script_consumed(&runner.bus.bus);
    }

    /// Section 6.2.2: the Output register carries a report in the same framing as SET_REPORT.
    #[tokio::test]
    async fn output_register_applies_spec_framed_report() {
        let (_request, bus) = Transaction::write(
            &[
                0x04, 0x00, // Output register address, little-endian
                0x06, 0x00, // wLength = 2 (self) + 1 (report ID) + 3 (payload)
                0x03, // report ID 3, explicit per MOUSE_DESCRIPTOR
                0xaa, 0xbb, 0xcc, // report payload
            ],
            WriteStatus::Stopped(8),
        )
        .finish();
        let mut resources = Resources::default();
        let mut runner = runner_with(&mut resources, bus, recording_device()).await;

        watchdog(runner.process_register_access()).await.unwrap();

        assert_eq!(runner.hid_device.report_id, Some(ReportId(3)));
        assert_eq!(
            runner.hid_device.report_data.get(..runner.hid_device.report_len),
            Some(&[0xaa, 0xbb, 0xcc][..])
        );
        assert!(!runner.hid_device.feature_report);
        assert_eq!(runner.bus.bus.recover_count, 0);
        assert_script_consumed(&runner.bus.bus);
    }

    /// End-to-end proof of the `WriteBufferSize = MaxOutputOrFeatureSize + 10` sizing: the
    /// largest SET_REPORT a host can legally send in one uninterrupted write - extended report
    /// ID in the command *and* the maximum payload - must fit the real `write_buf` without the
    /// bus ever reporting `BufferFull`.
    #[tokio::test]
    async fn set_report_max_size_with_extended_id_fits_shell_buffer() {
        let (_request, bus) = Transaction::write(
            &[
                0x05,
                0x00, // Command register address, little-endian
                0x3f, // report type Feature (0x3), report ID nibble 0xF = extended
                Opcode::SetReport as u8,
            ],
            WriteStatus::Stopped(4),
        )
        .append_to_initial_write(&[
            0x21, // "Third Byte": the extended report ID, appended to the command
            0x06, 0x00, // Data register address, little-endian
            0x0b, 0x00, // wLength = 2 (self) + 1 (report ID) + 8 (payload)
            0x21, // report ID, repeated at the head of the data payload per 7.2.3.1
            0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, // maximum 8-byte payload
        ])
        .finish();
        let mut resources = Resources::default();
        let mut runner = runner_with(&mut resources, bus, recording_device()).await;

        watchdog(runner.process_register_access()).await.unwrap();

        assert_eq!(runner.hid_device.report_id, Some(ReportId(0x21)));
        assert_eq!(
            runner.hid_device.report_data.get(..runner.hid_device.report_len),
            Some(&[0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88][..])
        );
        assert!(runner.hid_device.feature_report);
        assert_eq!(runner.bus.bus.recover_count, 0);
        assert_script_consumed(&runner.bus.bus);
    }

    /// A GET_REPORT answer is `[wLength(2)][report ID][payload]` with `wLength` counting itself,
    /// so a 1-byte payload behind an explicit report ID gives `wLength = 4`.
    #[tokio::test]
    async fn get_report_writes_header_then_payload() {
        let (_request, bus) = Transaction::write(
            &[
                0x05,
                0x00, // Command register address
                0x3f, // report type Feature (0x3), report ID nibble 0xF = extended
                Opcode::GetReport as u8,
                0x21, // extended report ID
                0x06,
                0x00, // Data register address
            ],
            WriteStatus::Stopped(7),
        )
        .then_read([
            // wLength = 2 (self) + 1 (report ID) + 1 (payload) = 4, then the report ID.
            (&[0x04, 0x00, 0x21][..], ReadStatus::Complete(3)),
            (&[0x5a][..], ReadStatus::Complete(1)),
        ])
        .finish();
        let mut resources = Resources::default();
        let device = recording_device().with_get_report_payload(&[0x5a]);
        let mut runner = runner_with(&mut resources, bus, device).await;

        watchdog(runner.process_register_access()).await.unwrap();

        assert_outgoing_reads(&runner.bus.bus.outgoing_reads, &[&[0x04, 0x00, 0x21], &[0x5a]]);
        assert_eq!(runner.bus.bus.recover_count, 0);
        assert_script_consumed(&runner.bus.bus);
    }

    /// GET_REPORT must be completed by a repeated start into a read. Anything else is a
    /// malformed command sequence, and nothing may be put on the wire.
    #[tokio::test]
    async fn get_report_non_read_followup_is_invalid_command() {
        let (_request, bus) = Transaction::write(
            &[
                0x05,
                0x00,
                0x3f,
                Opcode::GetReport as u8,
                0x21,
                HidI2cRegister::Data as u8,
                0x00,
            ],
            WriteStatus::Stopped(7),
        )
        .then_request(Request::Stop(HOST_ADDR))
        .finish();
        let mut resources = Resources::default();
        let mut runner = runner_with(&mut resources, bus, recording_device()).await;

        let result = watchdog(runner.process_register_access()).await;

        assert!(matches!(result, Err(Error::Protocol(ProtocolError::InvalidCommand))));
        assert_outgoing_reads(&runner.bus.bus.outgoing_reads, &[]);
        assert_script_consumed(&runner.bus.bus);
    }

    /// The header and the payload are two separate bus writes. A failure on the second leaves
    /// the first already on the wire - the host sees a truncated response, not nothing at all.
    #[tokio::test]
    async fn get_report_payload_bus_error_occurs_after_header() {
        let (_request, bus) = Transaction::write(
            &[
                0x05,
                0x00,
                0x3f,
                Opcode::GetReport as u8,
                0x21,
                HidI2cRegister::Data as u8,
                0x00,
            ],
            WriteStatus::Stopped(7),
        )
        .then_read([(&[0x04, 0x00, 0x21][..], ReadStatus::Complete(3))])
        .fail_next(ScriptOp::RespondToRead, ErrorKind::Bus)
        .finish();
        let mut resources = Resources::default();
        let device = recording_device().with_get_report_payload(&[0x5a]);
        let mut runner = runner_with(&mut resources, bus, device).await;

        let result = watchdog(runner.process_register_access()).await;

        assert!(matches!(result, Err(Error::Bus(ErrorKind::Bus))));
        assert_outgoing_reads(&runner.bus.bus.outgoing_reads, &[&[0x04, 0x00, 0x21]]);
        assert_script_consumed(&runner.bus.bus);
    }

    /// Section 5.1: a read of the device-descriptor register returns the descriptor verbatim.
    ///
    /// The expectation is written out literally rather than derived from `DeviceDescriptor::new`.
    /// Building it with the constructor under test would make the oracle circular: a constructor
    /// that emitted the wrong bytes would be compared against its own wrong bytes and still pass.
    ///
    /// The bytes below are the section 5.1 field layout (each field little-endian `u16`, in the
    /// order declared by `DeviceDescriptor`) filled in from the fixtures:
    ///   wHIDDescLength      0x001e  - 13 `u16` fields plus 4 reserved bytes
    ///   bcdVersion          0x0100  - HID-over-I2C protocol version
    ///   wReportDescLength   0x0013  - `MOUSE_DESCRIPTOR` is 19 bytes
    ///   wReportDescRegister 0x0002
    ///   wInputRegister      0x0003
    ///   wMaxInputLength     0x000b  - 8-byte max input report + 3 framing bytes (explicit IDs)
    ///   wOutputRegister     0x0004
    ///   wMaxOutputLength    0x000b  - 8-byte max output report + 3 framing bytes
    ///   wCommandRegister    0x0005
    ///   wDataRegister       0x0006
    ///   wVendorId           0x1234, wProductId 0x5678, wVersionId 0x0100 - `hardware_version_info()`
    ///   reserved            four zero bytes
    #[tokio::test]
    async fn device_descriptor_register_returns_descriptor() {
        const EXPECTED_DESCRIPTOR: &[u8] = &[
            0x1e, 0x00, // wHIDDescLength
            0x00, 0x01, // bcdVersion
            0x13, 0x00, // wReportDescLength
            0x02, 0x00, // wReportDescRegister
            0x03, 0x00, // wInputRegister
            0x0b, 0x00, // wMaxInputLength
            0x04, 0x00, // wOutputRegister
            0x0b, 0x00, // wMaxOutputLength
            0x05, 0x00, // wCommandRegister
            0x06, 0x00, // wDataRegister
            0x34, 0x12, // wVendorId
            0x78, 0x56, // wProductId
            0x00, 0x01, // wVersionId
            0x00, 0x00, 0x00, 0x00, // reserved
        ];

        let (_request, bus) = Transaction::write(&[0x01, 0x00], WriteStatus::Stopped(2))
            .then_read([(EXPECTED_DESCRIPTOR, ReadStatus::Complete(EXPECTED_DESCRIPTOR.len()))])
            .finish();
        let mut resources = Resources::default();
        let mut runner = runner_with(&mut resources, bus, recording_device()).await;

        watchdog(runner.process_register_access()).await.unwrap();

        assert_eq!(runner.device_descriptor.as_bytes(), EXPECTED_DESCRIPTOR);
        assert_outgoing_reads(&runner.bus.bus.outgoing_reads, &[EXPECTED_DESCRIPTOR]);
        assert_eq!(runner.bus.bus.recover_count, 0);
        assert_script_consumed(&runner.bus.bus);
    }

    #[tokio::test]
    async fn device_descriptor_rejects_non_read_followup() {
        let (_request, bus) = Transaction::write(&[0x01, 0x00], WriteStatus::Stopped(2))
            .then_request(Request::Stop(HOST_ADDR))
            .finish();
        let mut resources = Resources::default();
        let mut runner = runner_with(&mut resources, bus, recording_device()).await;

        let result = watchdog(runner.process_register_access()).await;

        assert!(matches!(
            result,
            Err(Error::Protocol(ProtocolError::InvalidRegisterAddress))
        ));
        assert_outgoing_reads(&runner.bus.bus.outgoing_reads, &[]);
        assert_script_consumed(&runner.bus.bus);
    }

    /// Section 5.2: a read of the report-descriptor register returns the device's own descriptor.
    #[tokio::test]
    async fn report_descriptor_register_returns_descriptor() {
        let expected = crate::test_support::MOUSE_DESCRIPTOR;
        let (_request, bus) = Transaction::write(&[0x02, 0x00], WriteStatus::Stopped(2))
            .then_read([(expected, ReadStatus::Complete(expected.len()))])
            .finish();
        let mut resources = Resources::default();
        let mut runner = runner_with(&mut resources, bus, recording_device()).await;

        watchdog(runner.process_register_access()).await.unwrap();

        assert_eq!(expected.len(), 19);
        assert_outgoing_reads(&runner.bus.bus.outgoing_reads, &[expected]);
        assert_eq!(runner.bus.bus.recover_count, 0);
        assert_script_consumed(&runner.bus.bus);
    }

    #[tokio::test]
    async fn report_descriptor_rejects_non_read_followup() {
        let (_request, bus) = Transaction::write(&[0x02, 0x00], WriteStatus::Stopped(2))
            .then_request(Request::Stop(HOST_ADDR))
            .finish();
        let mut resources = Resources::default();
        let mut runner = runner_with(&mut resources, bus, recording_device()).await;

        let result = watchdog(runner.process_register_access()).await;

        assert!(matches!(
            result,
            Err(Error::Protocol(ProtocolError::InvalidRegisterAddress))
        ));
        assert_outgoing_reads(&runner.bus.bus.outgoing_reads, &[]);
        assert_script_consumed(&runner.bus.bus);
    }

    /// Section 6.1: addressing the Input register and restarting into a read yields the queued
    /// input report, framed as `[wLength(2)][report ID][payload]`.
    #[tokio::test]
    async fn input_register_returns_pending_input_report() {
        let (_request, bus) = Transaction::write(&[0x03, 0x00], WriteStatus::Stopped(2))
            .then_read([
                // wLength = 2 (self) + 1 (report ID) + 2 (payload) = 5
                (&[0x05, 0x00, 0x03][..], ReadStatus::Complete(3)),
                (&[0xde, 0xad][..], ReadStatus::Complete(2)),
            ])
            .finish();
        let mut resources = Resources::default();
        let device = recording_device().with_pending_input(ReportId(3), &[0xde, 0xad]);
        let mut runner = runner_with(&mut resources, bus, device).await;

        watchdog(runner.process_register_access()).await.unwrap();

        assert_outgoing_reads(&runner.bus.bus.outgoing_reads, &[&[0x05, 0x00, 0x03], &[0xde, 0xad]]);
        assert!(runner.hid_device.pending_input.is_none());
        assert_eq!(runner.bus.bus.recover_count, 0);
        assert_script_consumed(&runner.bus.bus);
    }

    #[tokio::test]
    async fn input_register_rejects_non_read_followup() {
        let (_request, bus) = Transaction::write(&[0x03, 0x00], WriteStatus::Stopped(2))
            .then_request(Request::Stop(HOST_ADDR))
            .finish();
        let mut resources = Resources::default();
        let device = recording_device().with_pending_input(ReportId(3), &[0xde, 0xad]);
        let mut runner = runner_with(&mut resources, bus, device).await;

        let result = watchdog(runner.process_register_access()).await;

        assert!(matches!(result, Err(Error::Protocol(ProtocolError::InvalidCommand))));
        assert_outgoing_reads(&runner.bus.bus.outgoing_reads, &[]);
        assert_script_consumed(&runner.bus.bus);
    }

    /// Section 7.1.1: a write to the Command register is dispatched to the command decoder.
    #[tokio::test]
    async fn command_register_dispatches_command() {
        let (_request, bus) = Transaction::write(
            &[
                0x05,
                0x00, // Command register address, little-endian
                0x01, // command low byte: power state Sleep
                Opcode::SetPower as u8,
            ],
            WriteStatus::Stopped(4),
        )
        .finish();
        let mut resources = Resources::default();
        let mut runner = runner_with(&mut resources, bus, recording_device()).await;

        watchdog(runner.process_register_access()).await.unwrap();

        assert_eq!(runner.hid_device.power_state, Some(HidDevicePowerState::Sleep));
        assert_eq!(runner.bus.bus.recover_count, 0);
        assert_script_consumed(&runner.bus.bus);
    }

    /// Section 7.1.2: the Data register is only ever addressed as part of a command sequence.
    /// A bare write to it has no command to belong to.
    #[tokio::test]
    async fn data_register_without_preceding_command_is_rejected() {
        let (_request, bus) =
            Transaction::write(&[0x06, 0x00, 0x04, 0x00, 0x03, 0x5a], WriteStatus::Stopped(6)).finish();
        let mut resources = Resources::default();
        let mut runner = runner_with(&mut resources, bus, recording_device()).await;

        let result = watchdog(runner.process_register_access()).await;

        assert!(matches!(
            result,
            Err(Error::Protocol(ProtocolError::InvalidRegisterAddress))
        ));
        assert_eq!(runner.hid_device.report_id, None);
        assert_script_consumed(&runner.bus.bus);
    }

    /// A register address is two bytes; anything shorter is not addressing anything.
    #[tokio::test]
    async fn register_access_rejects_short_address() {
        let (_request, one_byte) = Transaction::write(&[0x01], WriteStatus::Stopped(1)).finish();
        let mut resources = Resources::default();
        let mut runner = runner_with(&mut resources, one_byte, recording_device()).await;

        let result = watchdog(runner.process_register_access()).await;

        assert!(matches!(result, Err(Error::Protocol(ProtocolError::InvalidData))));
        assert_script_consumed(&runner.bus.bus);

        let (_request, empty) = Transaction::write(&[], WriteStatus::Stopped(0)).finish();
        let mut empty_resources = Resources::default();
        let mut empty_runner = runner_with(&mut empty_resources, empty, recording_device()).await;

        let result = watchdog(empty_runner.process_register_access()).await;

        assert!(matches!(result, Err(Error::Protocol(ProtocolError::InvalidData))));
        assert_script_consumed(&empty_runner.bus.bus);
    }

    #[tokio::test]
    async fn register_access_rejects_unknown_register() {
        let (_request, bus) = Transaction::write(&[0x77, 0x77], WriteStatus::Stopped(2)).finish();
        let mut resources = Resources::default();
        let mut runner = runner_with(&mut resources, bus, recording_device()).await;

        let result = watchdog(runner.process_register_access()).await;

        assert!(matches!(
            result,
            Err(Error::Protocol(ProtocolError::InvalidRegisterAddress))
        ));
        assert_script_consumed(&runner.bus.bus);
    }

    // ---------------------------------------------------------------------------------------
    // `process_request` dispatch (pass B).
    //
    // `process_request` is the run loop's only entry point for a host-initiated bus event. It
    // returns `()`, so its entire observable contract is: which sub-handler it routes to, and
    // what it does with each of the four `Result` arms it can get back. Everything below drives
    // it directly rather than through `run()`, which never returns.
    // ---------------------------------------------------------------------------------------

    /// `Request::Write` means the host is addressing a register, so the write must land on the
    /// device.
    #[tokio::test]
    async fn process_request_dispatches_write_to_register_access() {
        let (request, bus) = Transaction::write(
            &[
                0x04, 0x00, // Output register address, little-endian
                0x04, 0x00, // wLength = 2 (self) + 1 (report ID) + 1 (payload)
                0x03, // report ID 3, explicit per MOUSE_DESCRIPTOR
                0x5a, // report payload
            ],
            WriteStatus::Stopped(6),
        )
        .finish();
        let mut resources = Resources::default();
        let mut runner = runner_with(&mut resources, bus, recording_device()).await;

        watchdog(runner.process_request(request)).await;

        assert_eq!(runner.hid_device.report_id, Some(ReportId(3)));
        assert_eq!(
            runner.hid_device.report_data.get(..runner.hid_device.report_len),
            Some(&[0x5a][..])
        );
        assert_eq!(runner.hid_device.reset_count, 0);
        assert_script_consumed(&runner.bus.bus);
    }

    /// `Request::Read` outside a register sequence is the host polling the input register, so it
    /// must be answered from the input-report queue.
    #[tokio::test]
    async fn process_request_dispatches_read_to_input_report() {
        let (request, bus) = Transaction::read()
            .then_read([
                // wLength = 2 (self) + 1 (report ID) + 2 (payload) = 5
                (&[0x05, 0x00, 0x03][..], ReadStatus::Complete(3)),
                (&[0xde, 0xad][..], ReadStatus::Complete(2)),
            ])
            .finish();
        let mut resources = Resources::default();
        let device = recording_device().with_pending_input(ReportId(3), &[0xde, 0xad]);
        let mut runner = runner_with(&mut resources, bus, device).await;

        watchdog(runner.process_request(request)).await;

        assert_outgoing_reads(&runner.bus.bus.outgoing_reads, &[&[0x05, 0x00, 0x03], &[0xde, 0xad]]);
        assert!(runner.hid_device.pending_input.is_none());
        assert_script_consumed(&runner.bus.bus);
    }

    /// Asserts that a request type carries no side effects at all: nothing on the wire, nothing
    /// on the device, nothing on the GPIO, and no scripted step consumed.
    async fn assert_request_is_ignored(request: Request) {
        let mut resources = Resources::default();
        let mut runner = runner_with(&mut resources, ScriptedBus::default(), recording_device()).await;

        watchdog(runner.process_request(request)).await;

        assert_outgoing_reads(&runner.bus.bus.outgoing_reads, &[]);
        assert_eq!(runner.bus.bus.recover_count, 0);
        assert_eq!(runner.hid_device.report_id, None);
        assert_eq!(runner.hid_device.reset_count, 0);
        assert_eq!(runner.hid_device.power_state, None);
        assert!(!runner.pending_reset);
        assert!(!runner.attn_pin.asserted());
        // Only the deassert `AttnPinHandler::new` performs; the request drove nothing further.
        assert_pin_levels(&runner.attn_pin.pin().levels, &[PinLevel::High]);
        assert_script_consumed(&runner.bus.bus);
    }

    /// A `Stop` closes a transaction we have already fully handled; there is nothing left to do.
    #[tokio::test]
    async fn process_request_ignores_stop() {
        assert_request_is_ignored(Request::Stop(HOST_ADDR)).await;
    }

    /// A bare `RepeatedStart` only ends the previous sub-transaction - the direction of the new
    /// one arrives on the next `listen`, so this edge alone is not actionable.
    #[tokio::test]
    async fn process_request_ignores_repeated_start() {
        assert_request_is_ignored(Request::RepeatedStart(HOST_ADDR)).await;
    }

    /// A general call is addressed to every target on the bus, not to us specifically.
    #[tokio::test]
    async fn process_request_ignores_general_call() {
        assert_request_is_ignored(Request::GeneralCall).await;
    }

    /// A bus-layer failure is logged and dropped: the run loop must keep servicing the host, and
    /// a bus error is not grounds for resetting the HID device.
    #[tokio::test]
    async fn process_request_swallows_bus_error() {
        let bus = ScriptedBus {
            respond_to_write_steps: VecDeque::from([RespondToWriteStep::Error(ErrorKind::Overrun)]),
            ..Default::default()
        };
        let mut resources = Resources::default();
        let mut runner = runner_with(&mut resources, bus, recording_device()).await;

        watchdog(runner.process_request(Request::Write(HOST_ADDR))).await;

        assert_eq!(runner.hid_device.reset_count, 0);
        assert_eq!(runner.hid_device.report_id, None);
        assert!(!runner.pending_reset);
        assert!(!runner.attn_pin.asserted());
        assert_script_consumed(&runner.bus.bus);
    }

    /// A malformed frame from the host is likewise logged and dropped - the host is free to
    /// retry, and resetting the device would punish it for the host's mistake.
    #[tokio::test]
    async fn process_request_swallows_protocol_error() {
        // A register address is two bytes; one byte cannot address anything.
        let (request, bus) = Transaction::write(&[0x01], WriteStatus::Stopped(1)).finish();
        let mut resources = Resources::default();
        let mut runner = runner_with(&mut resources, bus, recording_device()).await;

        watchdog(runner.process_request(request)).await;

        assert_eq!(runner.hid_device.reset_count, 0);
        assert_eq!(runner.hid_device.report_id, None);
        assert!(!runner.pending_reset);
        assert!(!runner.attn_pin.asserted());
        assert_script_consumed(&runner.bus.bus);
    }

    /// The device-initiated reset path: `HidError::TriggerReset` bubbling out of the device is
    /// the one error arm that is *not* swallowed. It must reset the device, latch the pending
    /// acknowledgement, and raise ATTN so the host comes back to collect it (spec section 7.2.1).
    #[tokio::test]
    async fn process_request_resets_on_device_trigger_reset() {
        let (request, bus) = Transaction::write(
            &[
                0x04, 0x00, // Output register address, little-endian
                0x04, 0x00, // wLength = 2 (self) + 1 (report ID) + 1 (payload)
                0x03, // report ID 3
                0x5a, // report payload
            ],
            WriteStatus::Stopped(6),
        )
        .finish();
        let mut resources = Resources::default();
        let mut device = recording_device();
        device.fail_set_report = Some(HidError::TriggerReset);
        let mut runner = runner_with(&mut resources, bus, device).await;

        watchdog(runner.process_request(request)).await;

        assert_eq!(runner.hid_device.reset_count, 1);
        assert!(runner.pending_reset);
        assert!(runner.attn_pin.asserted());
        // Deassert at construction, then the assert the reset performs.
        assert_pin_levels(&runner.attn_pin.pin().levels, &[PinLevel::High, PinLevel::Low]);
        assert_script_consumed(&runner.bus.bus);
    }

    // ---------------------------------------------------------------------------------------
    // Reply paths.
    // ---------------------------------------------------------------------------------------

    /// Section 7.2.1: the first input read after a reset is answered with a zero-length report,
    /// which is what tells the host the reset completed. The interrupt is then released.
    #[tokio::test]
    async fn reset_read_acknowledges_with_zero_report() {
        let (request, bus) = Transaction::read()
            .then_read([(&[0x00, 0x00][..], ReadStatus::Complete(2))])
            .finish();
        let mut resources = Resources::default();
        let mut runner = runner_with(&mut resources, bus, recording_device()).await;

        watchdog(runner.reset()).await;
        assert!(runner.pending_reset);

        watchdog(runner.process_request(request)).await;

        assert!(!runner.pending_reset);
        assert!(!runner.attn_pin.asserted());
        assert_outgoing_reads(&runner.bus.bus.outgoing_reads, &[&[0x00, 0x00]]);
        // Deassert at construction, assert on reset, deassert once the host has collected it.
        assert_pin_levels(
            &runner.attn_pin.pin().levels,
            &[PinLevel::High, PinLevel::Low, PinLevel::High],
        );
        assert_script_consumed(&runner.bus.bus);
    }

    /// The same zero-length reply, reached by the *other* route: no reset is pending and the
    /// device simply has nothing queued. `reset_count == 0` and `pending_reset == false`
    /// throughout are what distinguish this from the reset-acknowledgement path above.
    #[tokio::test]
    async fn read_without_pending_input_returns_zero_report() {
        let (request, bus) = Transaction::read()
            .then_read([(&[0x00, 0x00][..], ReadStatus::Complete(2))])
            .finish();
        let mut resources = Resources::default();
        let device = recording_device();
        assert!(device.pending_input.is_none(), "this test needs the no-pending branch");
        let mut runner = runner_with(&mut resources, bus, device).await;

        assert!(!runner.pending_reset, "this test must not take the reset branch");

        watchdog(runner.process_request(request)).await;

        assert_eq!(runner.hid_device.reset_count, 0);
        assert!(!runner.pending_reset);
        assert_outgoing_reads(&runner.bus.bus.outgoing_reads, &[&[0x00, 0x00]]);
        // The clear is attempted even though the interrupt was never raised.
        assert_pin_levels(&runner.attn_pin.pin().levels, &[PinLevel::High, PinLevel::High]);
        assert!(!runner.attn_pin.asserted());
        assert_script_consumed(&runner.bus.bus);
    }

    /// An input report is emitted as two separate bus writes. If the host abandons the
    /// transaction between them, the report has already been taken out of the device's queue by
    /// `process_next_input_report` and is not put back - so the host sees a truncated frame and
    /// the report is gone. ATTN also stays asserted, because the clear is only reached on the
    /// success path.
    ///
    /// CHARACTERISATION, not a specification: this test pins current behaviour - silent input
    /// report loss on a mid-frame failure, with ATTN left asserted - so that a change to it is
    /// noticed. Whether losing the report is correct is an open design question, not a settled
    /// requirement.
    #[tokio::test]
    async fn input_report_payload_timeout_preserves_failure_state() {
        let bus = ScriptedBus {
            // Only the header is scripted. The payload write finds an empty queue and pends,
            // so the device-response timeout fires mid-frame.
            respond_to_read_steps: VecDeque::from([RespondToReadStep::Success(ExpectedRead::new(
                &[0x05, 0x00, 0x03],
                ReadStatus::Complete(3),
            ))]),
            ..Default::default()
        };
        let mut resources = Resources::default();
        let device = recording_device().with_pending_input(ReportId(3), &[0xde, 0xad]);
        let mut runner = runner_with(&mut resources, bus, device).await;

        runner.attn_pin.assert_interrupt().unwrap();
        assert!(runner.attn_pin.asserted());

        let result = watchdog(runner.reply_with_input_report()).await;

        assert!(matches!(result, Err(Error::Protocol(ProtocolError::Timeout))));
        assert_eq!(runner.bus.bus.recover_count, 1);
        // Only the header reached the host; the payload never did.
        assert_outgoing_reads(&runner.bus.bus.outgoing_reads, &[&[0x05, 0x00, 0x03]]);
        // ...and the report is already gone from the device, so it can never be re-sent.
        assert!(runner.hid_device.pending_input.is_none());
        // The clear is unreachable on this path, so the host is still being told to read.
        assert!(runner.attn_pin.asserted());
        assert_pin_levels(&runner.attn_pin.pin().levels, &[PinLevel::High, PinLevel::Low]);
        assert_script_consumed(&runner.bus.bus);
    }

    /// The report reached the host, so the transfer succeeded; a GPIO that then refuses to
    /// deassert must not turn that success into an error. The handler's `asserted` flag stays
    /// set, because `clear_interrupt` only clears it after the pin actually moved.
    #[tokio::test]
    async fn input_report_clear_attn_failure_is_swallowed() {
        let bus = ScriptedBus {
            respond_to_read_steps: VecDeque::from([
                RespondToReadStep::Success(ExpectedRead::new(&[0x05, 0x00, 0x03], ReadStatus::Complete(3))),
                RespondToReadStep::Success(ExpectedRead::new(&[0xde, 0xad], ReadStatus::Complete(2))),
            ]),
            ..Default::default()
        };
        let mut resources = Resources::default();
        let (_service, mut runner) = Service::new(
            &mut resources,
            bus,
            // Two transitions succeed - the constructor's deassert and the explicit assert
            // below - and the final deassert fails.
            RecordingPin::failing_after(2),
            recording_device().with_pending_input(ReportId(3), &[0xde, 0xad]),
            hardware_version_info(),
            TimeoutSettings {
                device_response_timeout: Duration::from_millis(20),
                data_read_timeout: Duration::from_millis(20),
            },
        )
        .await
        .unwrap();

        runner.attn_pin.assert_interrupt().unwrap();

        let result = watchdog(runner.reply_with_input_report()).await;

        assert!(result.is_ok(), "a failed GPIO deassert must not fail the transfer");
        assert_outgoing_reads(&runner.bus.bus.outgoing_reads, &[&[0x05, 0x00, 0x03], &[0xde, 0xad]]);
        assert!(runner.hid_device.pending_input.is_none());
        // The deassert never happened, so the handler still believes it is asserting.
        assert!(runner.attn_pin.asserted());
        assert_pin_levels(&runner.attn_pin.pin().levels, &[PinLevel::High, PinLevel::Low]);
        assert_script_consumed(&runner.bus.bus);
    }

    // ---------------------------------------------------------------------------------------
    // `discard_remaining_bytes_from_host` and recovery precedence.
    //
    // The discard loop runs when the host has written more than our buffer can hold. It has to
    // drain the rest of the transaction without stalling the bus, and its error handling has a
    // precedence rule: a failing `recover()` outranks the timeout that provoked it, because the
    // caller can retry after a timeout but not after an unrecoverable bus.
    // ---------------------------------------------------------------------------------------

    /// A bus error during the drain is reported as-is; nothing is recovered, because the
    /// peripheral answered rather than went silent.
    #[tokio::test]
    async fn discard_remaining_bytes_propagates_bus_error() {
        let mut bus = scripted_timeout_bus(ScriptedBus {
            respond_to_write_steps: VecDeque::from([RespondToWriteStep::Error(ErrorKind::Bus)]),
            ..Default::default()
        });

        let result = watchdog(bus.discard_remaining_bytes_from_host()).await;

        assert!(matches!(result, Err(Error::Bus(ErrorKind::Bus))));
        assert_eq!(bus.bus.recover_count, 0);
        assert_script_consumed(&bus.bus);
    }

    /// A host that stops driving mid-drain leaves the bus hung, so it is recovered and the
    /// timeout reported. This reaches both statements of the timeout arm.
    #[tokio::test]
    async fn discard_timeout_recovers_then_reports_protocol_timeout() {
        let mut bus = scripted_timeout_bus(ScriptedBus {
            // One full buffer keeps the loop going; the next iteration finds an empty queue.
            respond_to_write_steps: VecDeque::from([RespondToWriteStep::Success(IncomingWrite {
                data: Vec::new(),
                status: WriteStatus::BufferFull(0),
            })]),
            ..Default::default()
        });

        let result = watchdog(bus.discard_remaining_bytes_from_host()).await;

        assert!(matches!(result, Err(Error::Protocol(ProtocolError::Timeout))));
        assert_eq!(bus.bus.recover_count, 1);
        assert_script_consumed(&bus.bus);
    }

    /// When recovery itself fails, the recovery error is what the caller sees: the `?` on
    /// `recover()` returns before the `Timeout` line is ever reached.
    #[tokio::test]
    async fn discard_timeout_recovery_failure_takes_precedence() {
        let mut bus = scripted_timeout_bus(ScriptedBus {
            respond_to_write_steps: VecDeque::from([RespondToWriteStep::Success(IncomingWrite {
                data: Vec::new(),
                status: WriteStatus::BufferFull(0),
            })]),
            fail_recover: Some(ErrorKind::Bus),
            ..Default::default()
        });

        let result = watchdog(bus.discard_remaining_bytes_from_host()).await;

        assert!(
            matches!(result, Err(Error::Bus(ErrorKind::Bus))),
            "the recovery failure must outrank the timeout that provoked it"
        );
        assert_eq!(bus.bus.recover_count, 1);
        assert_script_consumed(&bus.bus);
    }

    /// The drain loops until the host actually terminates the transaction, however many buffers
    /// that takes. A terminating status is not an error, so nothing is recovered.
    #[tokio::test]
    async fn discard_drains_multiple_full_buffers() {
        let mut bus = scripted_timeout_bus(ScriptedBus {
            respond_to_write_steps: VecDeque::from([
                RespondToWriteStep::Success(IncomingWrite {
                    data: vec![0x11, 0x22],
                    status: WriteStatus::BufferFull(2),
                }),
                RespondToWriteStep::Success(IncomingWrite {
                    data: vec![0x33, 0x44],
                    status: WriteStatus::BufferFull(2),
                }),
                RespondToWriteStep::Success(IncomingWrite {
                    data: vec![0x55],
                    status: WriteStatus::Stopped(1),
                }),
            ]),
            ..Default::default()
        });

        watchdog(bus.discard_remaining_bytes_from_host()).await.unwrap();

        assert_script_consumed(&bus.bus);
        assert_eq!(bus.bus.recover_count, 0);
    }

    /// The same drain reached the way production reaches it - through an oversized host write
    /// into `read` - so the `?` that propagates the drain's failure out of `read` is covered too.
    #[tokio::test]
    async fn read_oversize_discard_timeout_recovers() {
        let mut bus = scripted_timeout_bus(ScriptedBus {
            respond_to_write_steps: VecDeque::from([
                // The initial read overflows our buffer...
                RespondToWriteStep::Success(IncomingWrite {
                    data: vec![0x10, 0x20],
                    status: WriteStatus::BufferFull(2),
                }),
                // ...the first drain iteration overflows too, and then the host goes silent.
                RespondToWriteStep::Success(IncomingWrite {
                    data: vec![0x30, 0x40],
                    status: WriteStatus::BufferFull(2),
                }),
            ]),
            ..Default::default()
        });
        let mut buffer = [0; 2];

        let result = watchdog(bus.read(&mut buffer)).await;

        assert!(matches!(result, Err(Error::Protocol(ProtocolError::Timeout))));
        assert_eq!(bus.bus.recover_count, 1);
        assert_script_consumed(&bus.bus);
    }

    // ---------------------------------------------------------------------------------------
    // `listen_for_response`, `listen_indefinitely`, `TimeoutBus::read`, `write_unterminated`.
    // ---------------------------------------------------------------------------------------

    /// A repeated start only ends the previous sub-transaction; the direction we are waiting for
    /// arrives on the following `listen`, so the edge must be skipped rather than returned.
    #[tokio::test]
    async fn listen_for_response_skips_repeated_start() {
        let mut bus = scripted_timeout_bus(ScriptedBus {
            listen_steps: VecDeque::from([
                ListenStep::Success(Request::RepeatedStart(HOST_ADDR)),
                ListenStep::Success(Request::Read(HOST_ADDR)),
            ]),
            ..Default::default()
        });

        let result = watchdog(bus.listen_for_response()).await;

        assert!(matches!(result, Ok(Request::Read(address)) if address == HOST_ADDR));
        assert_script_consumed(&bus.bus);
        assert_eq!(bus.bus.recover_count, 0);
    }

    #[tokio::test]
    async fn listen_for_response_propagates_bus_error() {
        let mut bus = scripted_timeout_bus(ScriptedBus {
            listen_steps: VecDeque::from([ListenStep::Error(ErrorKind::Bus)]),
            ..Default::default()
        });

        let result = watchdog(bus.listen_for_response()).await;

        assert!(matches!(result, Err(Error::Bus(ErrorKind::Bus))));
        assert_script_consumed(&bus.bus);
    }

    /// The idle wait has no deadline: between transactions there is nothing to time out, so a
    /// request that arrives is simply returned.
    #[tokio::test]
    async fn listen_indefinitely_returns_next_request() {
        let mut bus = scripted_timeout_bus(ScriptedBus {
            listen_steps: VecDeque::from([ListenStep::Success(Request::Write(HOST_ADDR))]),
            ..Default::default()
        });

        let result = watchdog(bus.listen_indefinitely()).await;

        assert!(matches!(result, Ok(Request::Write(address)) if address == HOST_ADDR));
        assert_script_consumed(&bus.bus);
        assert_eq!(bus.bus.recover_count, 0);
    }

    /// A host that restarts instead of stopping has still finished writing, so the bytes it
    /// delivered are handed back exactly as they are for a `Stopped` transfer.
    #[tokio::test]
    async fn timeout_bus_read_accepts_restarted_write() {
        let mut bus = scripted_timeout_bus(ScriptedBus {
            respond_to_write_steps: VecDeque::from([RespondToWriteStep::Success(IncomingWrite {
                data: vec![0x10, 0x20, 0x30],
                status: WriteStatus::Restarted(3),
            })]),
            ..Default::default()
        });
        let mut buffer = [0; 4];

        let payload = watchdog(bus.read(&mut buffer)).await.unwrap();

        assert_eq!(payload, &[0x10, 0x20, 0x30]);
        assert_eq!(bus.bus.recover_count, 0);
    }

    /// A HAL that reports more bytes than the buffer could possibly hold is lying. The service
    /// must reject the claim rather than slice past the end of its own buffer.
    #[tokio::test]
    async fn timeout_bus_rejects_impossible_byte_count() {
        let mut bus = scripted_timeout_bus(ScriptedBus {
            respond_to_write_steps: VecDeque::from([RespondToWriteStep::Success(IncomingWrite {
                data: vec![0x10, 0x20],
                // 999 bytes into a 2-byte buffer is not physically possible.
                status: WriteStatus::Stopped(999),
            })]),
            ..Default::default()
        });
        let mut buffer = [0; 2];

        let result = watchdog(bus.read(&mut buffer)).await;

        assert!(matches!(result, Err(Error::Protocol(ProtocolError::InvalidData))));
        assert_eq!(bus.bus.recover_count, 0);
        assert_script_consumed(&bus.bus);
    }

    /// The write path applies the same precedence rule as the drain: a failing `recover()`
    /// replaces the timeout that triggered it.
    #[tokio::test]
    async fn write_unterminated_reports_recovery_failure() {
        let mut bus = scripted_timeout_bus(ScriptedBus {
            // Empty read queue, so the host never takes the bytes and the deadline fires.
            fail_recover: Some(ErrorKind::Bus),
            ..Default::default()
        });

        let result = watchdog(bus.write_unterminated(&[0xaa])).await;

        assert!(matches!(result, Err(Error::Bus(ErrorKind::Bus))));
        assert_eq!(bus.bus.recover_count, 1);
        assert_outgoing_reads(&bus.bus.outgoing_reads, &[]);
    }

    // ---------------------------------------------------------------------------------------
    // Construction and reset.
    // ---------------------------------------------------------------------------------------

    /// The `Service` handle and the `Runner` are separate objects; `Service::reset` is the only
    /// way a caller can ask for a device-initiated reset, and it reaches the runner through the
    /// shared signal the run loop selects on.
    #[tokio::test]
    async fn service_reset_signals_runner_resource() {
        let mut resources = Resources::default();
        let (mut service, runner) = Service::new(
            &mut resources,
            ScriptedBus::default(),
            RecordingPin::new(),
            recording_device(),
            hardware_version_info(),
            TimeoutSettings::default(),
        )
        .await
        .unwrap();

        service.reset();

        // `Signal` is single-slot and latched, so signalling before waiting cannot lose the
        // wakeup; the watchdog turns a regression here into a failure rather than a hang.
        watchdog(runner.resources.reset_signal.wait()).await;
    }

    /// `DeviceDescriptor::new` holds the device to its own `MAX_DESCRIPTOR_LEN` contract, and
    /// `Service::new` must surface that failure rather than construct a runner that would
    /// advertise a `wReportDescLength` it cannot honour.
    #[tokio::test]
    async fn service_new_propagates_descriptor_error() {
        let mut resources: Resources<ScriptedBus, RecordingPin, crate::test_support::UnderDeclaredDescriptorDevice> =
            Resources::default();

        let result = Service::new(
            &mut resources,
            ScriptedBus::default(),
            RecordingPin::new(),
            crate::test_support::under_declared_descriptor_device(),
            hardware_version_info(),
            TimeoutSettings::default(),
        )
        .await;

        // MOUSE_DESCRIPTOR is 19 bytes; the device declares a 4-byte upper bound.
        assert_eq!(
            result.err(),
            Some(crate::DeviceDescriptorError::ReportDescriptorTooLarge { actual: 19, max: 4 })
        );
    }

    /// The constructor deasserts the pin on a best-effort basis. A GPIO that cannot be driven at
    /// all must not make the handler unconstructible - there would be no way to bring the service
    /// up, and no way to log the problem.
    #[test]
    fn attn_constructor_tolerates_initial_clear_failure() {
        let handler = AttnPinHandler::new(RecordingPin::failing_after(0));

        assert!(!handler.asserted());
        // The drive was attempted and refused, so nothing was recorded.
        assert_pin_levels(&handler.pin().levels, &[]);
    }
}
