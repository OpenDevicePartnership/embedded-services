use bitfield::bitfield;
use core::convert::Infallible;
use embedded_services::{
    comms,
    relay::{SerializableMessage, SerializableResponse},
};
use mctp_rs::smbus_espi::SmbusEspiMedium;
use mctp_rs::{MctpMedium, MctpMessageHeaderTrait, MctpMessageTrait, MctpPacketError, MctpPacketResult};

#[derive(num_enum::IntoPrimitive, num_enum::TryFromPrimitive, Debug, PartialEq, Eq, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[repr(u8)]
pub(crate) enum OdpService {
    Battery = 0x08,
    Thermal = 0x09,
    Debug = 0x0A,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub(crate) enum MctpError {
    // The endpoint ID does not correspond to a known service
    UnknownEndpointId,
}

impl TryFrom<comms::EndpointID> for OdpService {
    type Error = MctpError;
    fn try_from(endpoint_id: comms::EndpointID) -> Result<Self, MctpError> {
        match endpoint_id {
            comms::EndpointID::Internal(comms::Internal::Battery) => Ok(OdpService::Battery),
            comms::EndpointID::Internal(comms::Internal::Thermal) => Ok(OdpService::Thermal),
            comms::EndpointID::Internal(comms::Internal::Debug) => Ok(OdpService::Debug),
            _ => Err(MctpError::UnknownEndpointId),
        }
    }
}

impl OdpService {
    pub fn get_endpoint_id(&self) -> comms::EndpointID {
        match self {
            OdpService::Battery => comms::EndpointID::Internal(comms::Internal::Battery),
            OdpService::Thermal => comms::EndpointID::Internal(comms::Internal::Thermal),
            OdpService::Debug => comms::EndpointID::Internal(comms::Internal::Debug),
        }
    }
}

// TODO We'd ideally like these types to be passed in as a generic or something when the eSPI service is instantiated
//      so the eSPI service can be extended to handle 3rd party message types without needing to fork the eSPI service,
//      but that's dependant on us migrating to have storage for the eSPI service be allocated by the caller of init()
//      rather than statically allocated inside this module, so for now we accept this hardcoded list of supported message
//      types, and we can maybe convert this to a macro that accepts a list of types at some point.
//      New services should follow the pattern of defining their own message crates using the request/response
//      traits, and the only additions here should be the mapping between message types and endpoint IDs.
//
//      Additionally, we probably want some sort of macro that can generate most or all of this from a table mapping service IDs
//      to (request type, response type, comms endpoint) tuples for maintainability.
//
pub(crate) enum HostRequest {
    Battery(battery_service_messages::AcpiBatteryRequest),
    Debug(debug_service_messages::DebugRequest),
    Thermal(thermal_service_messages::ThermalRequest),
}

impl MctpMessageTrait<'_> for HostRequest {
    const MESSAGE_TYPE: u8 = 0x7D; // ODP message type

    type Header = OdpHeader;

    fn serialize<M: MctpMedium>(self, buffer: &mut [u8]) -> MctpPacketResult<usize, M> {
        match self {
            HostRequest::Battery(request) => request
                .serialize(buffer)
                .map_err(|_| mctp_rs::MctpPacketError::SerializeError("Failed to serialize battery request")),

            HostRequest::Debug(request) => request
                .serialize(buffer)
                .map_err(|_| mctp_rs::MctpPacketError::SerializeError("Failed to serialize debug request")),

            HostRequest::Thermal(request) => request
                .serialize(buffer)
                .map_err(|_| mctp_rs::MctpPacketError::SerializeError("Failed to serialize thermal request")),
        }
    }

    fn deserialize<M: MctpMedium>(header: &Self::Header, buffer: &'_ [u8]) -> MctpPacketResult<Self, M> {
        Ok(match header.service {
            OdpService::Battery => Self::Battery(
                battery_service_messages::AcpiBatteryRequest::deserialize(header.message_id, buffer)
                    .map_err(|_| MctpPacketError::CommandParseError("Could not parse battery request"))?,
            ),
            OdpService::Debug => Self::Debug(
                debug_service_messages::DebugRequest::deserialize(header.message_id, buffer)
                    .map_err(|_| MctpPacketError::CommandParseError("Could not parse debug request"))?,
            ),
            OdpService::Thermal => Self::Thermal(
                thermal_service_messages::ThermalRequest::deserialize(header.message_id, buffer)
                    .map_err(|_| MctpPacketError::CommandParseError("Could not parse thermal request"))?,
            ),
        })
    }
}

impl HostRequest {
    pub(crate) async fn send_to_endpoint(
        &self,
        source_endpoint: &comms::Endpoint,
        destination_endpoint_id: comms::EndpointID,
    ) -> Result<(), Infallible> {
        match self {
            HostRequest::Battery(request) => source_endpoint.send(destination_endpoint_id, request).await,
            HostRequest::Debug(request) => source_endpoint.send(destination_endpoint_id, request).await,
            HostRequest::Thermal(request) => source_endpoint.send(destination_endpoint_id, request).await,
        }
    }
}

#[derive(Clone)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub(crate) enum HostResponse {
    Battery(Result<battery_service_messages::AcpiBatteryResponse, battery_service_messages::AcpiBatteryError>),
    Debug(Result<debug_service_messages::DebugResponse, debug_service_messages::DebugError>),
    Thermal(Result<thermal_service_messages::ThermalResponse, thermal_service_messages::ThermalError>),
}

impl MctpMessageTrait<'_> for HostResponse {
    const MESSAGE_TYPE: u8 = 0x7D; // ODP message type

    type Header = OdpHeader;

    fn serialize<M: MctpMedium>(self, buffer: &mut [u8]) -> MctpPacketResult<usize, M> {
        match self {
            HostResponse::Battery(response) => response
                .serialize(buffer)
                .map_err(|_| mctp_rs::MctpPacketError::SerializeError("Failed to serialize battery response")),

            HostResponse::Debug(response) => response
                .serialize(buffer)
                .map_err(|_| mctp_rs::MctpPacketError::SerializeError("Failed to serialize debug response")),

            HostResponse::Thermal(response) => response
                .serialize(buffer)
                .map_err(|_| mctp_rs::MctpPacketError::SerializeError("Failed to serialize thermal response")),
        }
    }

    fn deserialize<M: MctpMedium>(header: &Self::Header, buffer: &'_ [u8]) -> MctpPacketResult<Self, M> {
        Ok(match header.service {
            OdpService::Battery => {
                if let Ok(success) =
                    battery_service_messages::AcpiBatteryResponse::deserialize(header.message_id, buffer)
                {
                    Self::Battery(Ok(success))
                } else {
                    let error = battery_service_messages::AcpiBatteryError::deserialize(header.message_id, buffer)
                        .map_err(|_| MctpPacketError::CommandParseError("Could not parse battery error response"))?;
                    Self::Battery(Err(error))
                }
            }
            OdpService::Debug => {
                if let Ok(success) = debug_service_messages::DebugResponse::deserialize(header.message_id, buffer) {
                    Self::Debug(Ok(success))
                } else {
                    let error = debug_service_messages::DebugError::deserialize(header.message_id, buffer)
                        .map_err(|_| MctpPacketError::CommandParseError("Could not parse debug error response"))?;
                    Self::Debug(Err(error))
                }
            }
            OdpService::Thermal => {
                if let Ok(success) = thermal_service_messages::ThermalResponse::deserialize(header.message_id, buffer) {
                    Self::Thermal(Ok(success))
                } else {
                    let error = thermal_service_messages::ThermalError::deserialize(header.message_id, buffer)
                        .map_err(|_| MctpPacketError::CommandParseError("Could not parse thermal error response"))?;
                    Self::Thermal(Err(error))
                }
            }
        })
    }
}

impl HostResponse {
    pub(crate) fn discriminant(&self) -> u16 {
        match self {
            HostResponse::Battery(response) => response.discriminant(),
            HostResponse::Debug(response) => response.discriminant(),
            HostResponse::Thermal(response) => response.discriminant(),
        }
    }

    pub(crate) fn is_ok(&self) -> bool {
        match self {
            HostResponse::Battery(response) => response.is_ok(),
            HostResponse::Debug(response) => response.is_ok(),
            HostResponse::Thermal(response) => response.is_ok(),
        }
    }
}

/// Attempt to route the provided message to the service that is registered to handle it based on its type.
pub(crate) fn try_route_request_to_comms(
    message: &comms::Message,
    send_fn: impl FnOnce(comms::EndpointID, HostResponse) -> Result<(), comms::MailboxDelegateError>,
) -> Result<(), comms::MailboxDelegateError> {
    // TODO we're going to have a bunch of types that all implement the SerializableResponse trait; in C++ I'd reach for dynamic_cast or a pointer-to-interface,
    //      but not sure how to do that with Rust's Any - it seems like it requires a concrete type rather than a trait to cast.  Is there a cleaner way to
    //      say "if the message implements the SerializableResponse trait" so we don't have to spell out all the types here?
    //
    if let Some(msg) = message
        .data
        .get::<Result<battery_service_messages::AcpiBatteryResponse, battery_service_messages::AcpiBatteryError>>()
    {
        send_fn(
            comms::EndpointID::Internal(comms::Internal::Battery),
            HostResponse::Battery(*msg),
        )?;
        Ok(())
    } else if let Some(msg) = message
        .data
        .get::<Result<debug_service_messages::DebugResponse, debug_service_messages::DebugError>>()
    {
        send_fn(
            comms::EndpointID::Internal(comms::Internal::Debug),
            HostResponse::Debug(*msg),
        )?;
        Ok(())
    } else if let Some(msg) = message
        .data
        .get::<Result<thermal_service_messages::ThermalResponse, thermal_service_messages::ThermalError>>()
    {
        send_fn(
            comms::EndpointID::Internal(comms::Internal::Thermal),
            HostResponse::Thermal(*msg),
        )?;
        Ok(())
    } else {
        Err(comms::MailboxDelegateError::MessageNotFound)
    }
}

bitfield! {
    /// Raw bitfield of possible port status events
    #[derive(Copy, Clone, PartialEq, Eq)]
    #[cfg_attr(feature = "defmt", derive(defmt::Format))]
    struct OdpHeaderWireFormat(u32);
    impl Debug;
    impl new;
    /// If true, represents a request; otherwise, represents a response
    is_request, set_is_request: 25;

    // TODO do we even want this bit? I think we just cribbed it off of a different message type, but it's not clear to me that we actually need it...
    is_datagram, set_is_datagram: 24;

    /// The service ID that this message is related to
    /// Note: Error checking is done when you access the field, not when you construct the OdpHeader. Take care when constructing a header.
    u8, service_id, set_service_id: 23, 16;

    /// On responses, indicates if the response message is an error. Unused on requests.
    is_error, set_is_error: 15;

    /// The message type/discriminant
    u16, message_id, set_message_id: 14, 0;

}

#[derive(Copy, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub(crate) enum OdpMessageType {
    Request,
    Response { is_error: bool },
}

#[derive(Copy, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub(crate) struct OdpHeader {
    pub message_type: OdpMessageType,
    pub is_datagram: bool, // TODO do we even want this bit? I think we just cribbed it off of a different message type, but it's not clear to me that we actually need it...
    pub service: OdpService,
    pub message_id: u16,
}

impl From<OdpHeader> for OdpHeaderWireFormat {
    fn from(src: OdpHeader) -> Self {
        Self::new(
            matches!(src.message_type, OdpMessageType::Request),
            src.is_datagram,
            src.service.into(),
            match src.message_type {
                OdpMessageType::Request => false, // unused on requests
                OdpMessageType::Response { is_error } => is_error,
            },
            src.message_id,
        )
    }
}

impl TryFrom<OdpHeaderWireFormat> for OdpHeader {
    type Error = MctpPacketError<SmbusEspiMedium>;

    fn try_from(src: OdpHeaderWireFormat) -> Result<Self, Self::Error> {
        let service = OdpService::try_from(src.service_id())
            .map_err(|_| MctpPacketError::HeaderParseError("invalid odp service in odp header"))?;

        let message_type = if src.is_request() {
            OdpMessageType::Request
        } else {
            OdpMessageType::Response {
                is_error: src.is_error(),
            }
        };

        Ok(OdpHeader {
            message_type,
            is_datagram: src.is_datagram(),
            service,
            message_id: src.message_id(),
        })
    }
}

impl MctpMessageHeaderTrait for OdpHeader {
    fn serialize<M: MctpMedium>(self, buffer: &mut [u8]) -> MctpPacketResult<usize, M> {
        let wire_format = OdpHeaderWireFormat::from(self);
        let bytes = wire_format.0.to_be_bytes();
        buffer
            .get_mut(0..bytes.len())
            .ok_or(MctpPacketError::SerializeError("buffer too small for odp header"))?
            .copy_from_slice(&bytes);

        Ok(bytes.len())
    }

    fn deserialize<M: MctpMedium>(buffer: &[u8]) -> MctpPacketResult<(Self, &[u8]), M> {
        let bytes = buffer
            .get(0..core::mem::size_of::<u32>())
            .ok_or(MctpPacketError::HeaderParseError("buffer too small for odp header"))?;
        let raw = u32::from_be_bytes(
            bytes
                .try_into()
                .map_err(|_| MctpPacketError::HeaderParseError("buffer too small for odp header"))?,
        );

        let parsed_wire_format = OdpHeaderWireFormat(raw);
        let header = OdpHeader::try_from(parsed_wire_format)
            .map_err(|_| MctpPacketError::HeaderParseError("invalid odp header received"))?;

        Ok((
            header,
            buffer
                .get(core::mem::size_of::<u32>()..)
                .ok_or(MctpPacketError::HeaderParseError("buffer too small for odp header"))?,
        ))
    }
}
