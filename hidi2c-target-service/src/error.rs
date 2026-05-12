use embassy_time::TimeoutError;
use embedded_services::relay::hid::HidError;

//  HID errors
#[derive(Clone, Copy, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub(crate) enum ProtocolError {
    /// Invalid data
    InvalidData,
    /// Invalid size
    InvalidSize,
    /// Invalid register address
    InvalidRegisterAddress,
    /// Invalid command
    InvalidCommand,
    /// Invalid report type for command
    InvalidReportType,
    /// Timeout
    Timeout,
}

#[allow(dead_code)] // Dead code analysis ignores Debug, which is what we want the detail for
#[derive(Clone, Copy, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub(crate) enum Error<BusError> {
    /// Error from the underlying bus
    Bus(BusError),
    /// HID protocol error
    Protocol(ProtocolError),
    /// Error reported from our underlying HidDevice in response to a request from us
    Device(HidError),
}

impl<BusError> From<ProtocolError> for Error<BusError> {
    fn from(err: ProtocolError) -> Self {
        Error::Protocol(err)
    }
}

impl<BusError> From<HidError> for Error<BusError> {
    fn from(err: HidError) -> Self {
        Error::Device(err)
    }
}

impl<BusError> From<generic_array::LengthError> for Error<BusError> {
    fn from(_: generic_array::LengthError) -> Self {
        Error::Protocol(ProtocolError::InvalidSize)
    }
}

impl<BusError> From<TimeoutError> for Error<BusError> {
    fn from(_: TimeoutError) -> Self {
        Error::Protocol(ProtocolError::Timeout)
    }
}
