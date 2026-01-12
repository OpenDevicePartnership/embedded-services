//! Helper code for serialization/deserialization of arbitrary messages to/from the embedded controller via a relay service, e.g. the eSPI service.

/// Error type for serializing/deserializing messages
#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum MessageSerializationError {
    /// The message payload does not represent a valid message
    InvalidPayload(&'static str),

    /// The message discriminant does not represent a known message type
    UnknownMessageDiscriminant(u16),

    /// The provided buffer is too small to serialize the message
    BufferTooSmall,

    /// Unspecified error
    Other(&'static str),
}

/// Trait for serializing and deserializing messages
pub trait SerializableMessage: Sized {
    /// Serializes the message into the provided buffer.
    /// On success, returns the number of bytes written
    fn serialize(self, buffer: &mut [u8]) -> Result<usize, MessageSerializationError>;

    ///  Returns the discriminant needed to deserialize this type of message.
    fn discriminant(&self) -> u16;

    /// Deserializes the message from the provided buffer.
    fn deserialize(discriminant: u16, buffer: &[u8]) -> Result<Self, MessageSerializationError>;
}

// Prevent other types from implementing SerializableResult - they should instead use SerializableMessage on a Response type and an Error type
#[doc(hidden)]
mod private {
    pub trait Sealed {}

    impl<T, E> Sealed for Result<T, E> {}
}

/// Responses are of type Result<T, E> where T and E both implement SerializableMessage
pub trait SerializableResult: private::Sealed + Sized {
    /// The type of the result when the operation being responded to succeeded
    type SuccessType: SerializableMessage;

    /// The type of the result when the operation being responded to failed
    type ErrorType: SerializableMessage;

    /// Returns true if the result represents a successful operation, false otherwise
    fn is_ok(&self) -> bool;

    /// Returns a unique discriminant that can be used to deserialize the specific type of result.
    /// Discriminants can be reused for success and error messages.
    fn discriminant(&self) -> u16;

    /// Writes the result into the provided buffer.
    /// On success, returns the number of bytes written
    fn serialize(self, buffer: &mut [u8]) -> Result<usize, MessageSerializationError>;

    /// Attempts to deserialize the result from the provided buffer.
    fn deserialize(is_error: bool, discriminant: u16, buffer: &[u8]) -> Result<Self, MessageSerializationError>;
}

impl<T, E> SerializableResult for Result<T, E>
where
    T: SerializableMessage,
    E: SerializableMessage,
{
    type SuccessType = T;
    type ErrorType = E;

    fn is_ok(&self) -> bool {
        Result::<T, E>::is_ok(self)
    }

    fn discriminant(&self) -> u16 {
        match self {
            Ok(success_value) => success_value.discriminant(),
            Err(error_value) => error_value.discriminant(),
        }
    }

    fn serialize(self, buffer: &mut [u8]) -> Result<usize, MessageSerializationError> {
        match self {
            Ok(success_value) => success_value.serialize(buffer),
            Err(error_value) => error_value.serialize(buffer),
        }
    }

    fn deserialize(is_error: bool, discriminant: u16, buffer: &[u8]) -> Result<Self, MessageSerializationError> {
        if is_error {
            Ok(Err(E::deserialize(discriminant, buffer)?))
        } else {
            Ok(Ok(T::deserialize(discriminant, buffer)?))
        }
    }
}

pub mod mctp {
    //! Contains helper functions for services that relay comms messages over MCTP

    /// Error type for MCTP relay operations
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    #[cfg_attr(feature = "defmt", derive(defmt::Format))]
    pub enum MctpError {
        /// The endpoint ID does not correspond to a known service
        UnknownEndpointId,
    }

    /// This macro generates the necessary types and impls to support relaying ODP messages to and from the comms system.
    /// It takes as input a list of (service name, service ID, comms endpoint ID, request type, result type) tuples and
    /// emits the following types:
    ///   - enum OdpService - a mapping from service name to MCTP endpoint ID
    ///   - enum HostRequest - an enum containing all the possible request types that were passed into the macro
    ///   - enum HostResult - an enum containing all the possible result types that were passed into the macro
    ///   - struct OdpHeader - a type representing the ODP MCTP header.
    ///   - fn send_to_comms(&comms::Message, impl FnOnce(comms::EndpointID, HostResult) -> Result<(), comms::MailboxDelegateError>,
    ///     a function that takes a received message and sends it to the appropriate service based on its type using the provided send function.
    ///
    /// Because this macro emits a number of types, it is recommended to invoke it inside a dedicated module.
    ///
    /// Arguments:
    ///    $service_name (identifier) - the name that this service will have in the emitted OdpService enum
    ///    $service_id (u8) - the service ID that will be used in the ODP MCTP header for messages related to this service.
    ///    $endpoint_id (comms::EndpointID value) - the comms endpoint ID that this service corresponds to.
    ///                                             NOTE: due to technical limitations in Rust macros, this must be surrounded with parentheses.
    ///    $request_type (type implementing SerializableMessage) - the type that represents requests for this service
    ///    $result_type (type implementing SerializableResult) - the type that represents results for this service
    ///
    /// Example usage:
    ///
    /// impl_odp_relay_types!(
    ///     Battery, 0x08, (comms::EndpointID::Internal(comms::Internal::Battery)), battery_service_messages::AcpiBatteryRequest, battery_service_messages::AcpiBatteryResult;
    ///     Thermal, 0x09, (comms::EndpointID::Internal(comms::Internal::Thermal)), thermal_service_messages::ThermalRequest, thermal_service_messages::ThermalResult;
    ///     Debug,   0x0A, (comms::EndpointID::Internal(comms::Internal::Debug)),   debug_service_messages::DebugRequest, debug_service_messages::DebugResult;
    /// );
    ///                    ^                                                   ^
    ///                    note the above parentheses - these are required
    #[macro_export]
    macro_rules! impl_odp_mctp_relay_types {
        ($($service_name:ident,
        $service_id:expr,
        ($($endpoint_id:tt)+),
        $request_type:ty,
        $result_type:ty;
        )+) => {

        use bitfield::bitfield;
        use core::convert::Infallible;
        use mctp_rs::smbus_espi::SmbusEspiMedium;
        use mctp_rs::{MctpMedium, MctpMessageHeaderTrait, MctpMessageTrait, MctpPacketError, MctpPacketResult};

        #[derive(num_enum::IntoPrimitive, num_enum::TryFromPrimitive, Debug, PartialEq, Eq, Clone, Copy)]
        #[cfg_attr(feature = "defmt", derive(defmt::Format))]
        #[repr(u8)]
        pub(crate) enum OdpService {
            $(
                $service_name = $service_id,
            )+
        }

        impl TryFrom<comms::EndpointID> for OdpService {
            type Error = embedded_services::relay::mctp::MctpError;
            fn try_from(endpoint_id_value: comms::EndpointID) -> Result<Self, embedded_services::relay::mctp::MctpError> {
                match endpoint_id_value {
                    $(
                        $($endpoint_id)+ => Ok(OdpService::$service_name),
                    )+
                    _ => Err(embedded_services::relay::mctp::MctpError::UnknownEndpointId),
                }
            }
        }

        impl OdpService {
            pub fn get_endpoint_id(&self) -> comms::EndpointID {
                match self {
                    $(
                        OdpService::$service_name => $($endpoint_id)+,
                    )+
                }
            }
        }

        pub(crate) enum HostRequest {
            $(
                $service_name($request_type),
            )+
        }

        impl HostRequest {
            pub(crate) async fn send_to_endpoint(&self, source_endpoint: &comms::Endpoint, destination_endpoint_id: comms::EndpointID) -> Result<(), Infallible> {
                match self {
                    $(
                        HostRequest::$service_name(request) => source_endpoint.send(destination_endpoint_id, request).await,
                    )+
                }
            }
        }

        impl MctpMessageTrait<'_> for HostRequest {
            type Header = OdpHeader;
            const MESSAGE_TYPE: u8 = 0x7D; // ODP message type

            fn serialize<M: MctpMedium>(self, buffer: &mut [u8]) -> MctpPacketResult<usize, M> {
                match self {
                    $(
                        HostRequest::$service_name(request) => request
                            .serialize(buffer)
                            .map_err(|_| mctp_rs::MctpPacketError::SerializeError(concat!("Failed to serialize ", stringify!($service_name), " request"))),
                    )+
                }
            }

            fn deserialize<M: MctpMedium>(header: &Self::Header, buffer: &'_ [u8]) -> MctpPacketResult<Self, M> {
                Ok(match header.service {
                    $(
                        OdpService::$service_name => Self::$service_name(
                            <$request_type>::deserialize(header.message_id, buffer)
                                .map_err(|_| MctpPacketError::CommandParseError(concat!("Could not parse ", stringify!($service_name), " request")))?,
                        ),
                    )+
                })
            }
        }

        #[derive(Clone)]
        #[cfg_attr(feature = "defmt", derive(defmt::Format))]
        pub(crate) enum HostResult {
            $(
                $service_name($result_type),
            )+
        }

        impl HostResult {
            pub(crate) fn discriminant(&self) -> u16 {
                match self {
                    $(
                        HostResult::$service_name(result) => result.discriminant(),
                    )+
                }
            }

            pub(crate) fn is_ok(&self) -> bool {
                match self {
                    $(
                        HostResult::$service_name(result) => result.is_ok(),
                    )+
                }
            }
        }

        impl MctpMessageTrait<'_> for HostResult {
            const MESSAGE_TYPE: u8 = 0x7D; // ODP message type

            type Header = OdpHeader;

            fn serialize<M: MctpMedium>(self, buffer: &mut [u8]) -> MctpPacketResult<usize, M> {
                match self {
                    $(
                        HostResult::$service_name(result) => result
                            .serialize(buffer)
                            .map_err(|_| mctp_rs::MctpPacketError::SerializeError(concat!("Failed to serialize ", stringify!($service_name), " result"))),
                    )+
                }
            }

            fn deserialize<M: MctpMedium>(header: &Self::Header, buffer: &'_ [u8]) -> MctpPacketResult<Self, M> {
                match header.service {
                    $(
                        OdpService::$service_name => {
                            match header.message_type {
                                OdpMessageType::Request => {
                                    Err(MctpPacketError::CommandParseError(concat!("Received ", stringify!($service_name), " request when expecting result")))
                                }
                                OdpMessageType::Result { is_error } => {
                                    Ok(HostResult::$service_name(<$result_type as SerializableResult>::deserialize(is_error, header.message_id, buffer)
                                        .map_err(|_| MctpPacketError::CommandParseError(concat!("Could not parse ", stringify!($service_name), " result")))?))
                                }
                            }
                        },
                    )+
                }
            }
        }

        bitfield! {
            /// Raw bitfield of possible port status events
            #[derive(Copy, Clone, PartialEq, Eq)]
            #[cfg_attr(feature = "defmt", derive(defmt::Format))]
            struct OdpHeaderWireFormat(u32);
            impl Debug;
            impl new;
            /// If true, represents a request; otherwise, represents a result
            is_request, set_is_request: 25;

            // TODO do we even want this bit? I think we just cribbed it off of a different message type, but it's not clear to me that we actually need it...
            is_datagram, set_is_datagram: 24;

            /// The service ID that this message is related to
            /// Note: Error checking is done when you access the field, not when you construct the OdpHeader. Take care when constructing a header.
            u8, service_id, set_service_id: 23, 16;

            /// On results, indicates if the result message is an error. Unused on requests.
            is_error, set_is_error: 15;

            /// The message type/discriminant
            u16, message_id, set_message_id: 14, 0;
        }

        #[derive(Copy, Clone, PartialEq, Eq)]
        #[cfg_attr(feature = "defmt", derive(defmt::Format))]
        pub(crate) enum OdpMessageType {
            Request,
            Result { is_error: bool },
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
                        OdpMessageType::Result { is_error } => is_error,
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
                    OdpMessageType::Result {
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

        /// Attempt to route the provided message to the service that is registered to handle it based on its type.
        pub(crate) fn send_to_comms(
            message: &comms::Message,
            send_fn: impl FnOnce(comms::EndpointID, HostResult) -> Result<(), comms::MailboxDelegateError>,
        ) -> Result<(), comms::MailboxDelegateError> {
            if (false) { unreachable!() }
            $(
                else if let Some(msg) = message.data.get::<$result_type>() {
                    send_fn(
                        $($endpoint_id)+,
                        HostResult::$service_name(*msg),
                    )?;
                    Ok(())
                }
            )+
            else {
                Err(comms::MailboxDelegateError::MessageNotFound)
            }
        }
    };
}

    pub use impl_odp_mctp_relay_types;
}
