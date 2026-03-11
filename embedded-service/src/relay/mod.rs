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

/// Responses sent over MCTP are called "Results" and are of type Result<T, E> where T and E both implement SerializableMessage
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

    /// Trait for types that are used by a relay service to relay messages from your service over the wire.
    /// If you are implementing this trait, you should also implement RelayServiceHandler.
    ///
    pub trait RelayServiceHandlerTypes {
        /// The request type that this service handler processes
        type RequestType: super::SerializableMessage;

        /// The result type that this service handler processes
        type ResultType: super::SerializableResult;

        /// The message type that this service broadcasts
        type MessageType;
    }

    /// Trait for a service that can be relayed over an external bus (e.g. battery service, thermal service, time-alarm service)
    ///
    pub trait RelayServiceHandler: RelayServiceHandlerTypes {
        /// Process the provided request and yield a result.
        fn process_request<'a>(
            &'a self,
            request: Self::RequestType,
        ) -> impl core::future::Future<Output = Self::ResultType> + 'a;

        /// Returns whether the given message should be treated as a relay notification.
        fn is_notification(message: &Self::MessageType) -> bool;
    }

    // Traits below this point are intended for consumption by relay services (e.g. the eSPI service), not individual services that want their messages relayed.
    // In general, you should not implement these yourself; rather, you should leverage the `impl_odp_mctp_relay_handler` macro to do that for you.

    /// Contains additional methods that must be implemented on the relay header type.
    /// Do not implement this yourself - rather, rely on the `impl_odp_mctp_relay_handler` macro to implement this.
    #[doc(hidden)]
    pub trait RelayHeader<ServiceIdType> {
        /// Return the ID of the service associated with the request
        fn get_service_id(&self) -> ServiceIdType;
    }

    /// Contains additional methods that must be implemented on the relay response type.
    /// Do not implement this yourself - rather, rely on the `impl_odp_mctp_relay_handler` macro to implement this.
    #[doc(hidden)]
    pub trait RelayResponse<ServiceIdType, HeaderType> {
        /// Construct an MCTP header suitable for representing the result based on the provided service handler ID and result
        fn create_header(&self, service_id: &ServiceIdType) -> HeaderType;
    }

    /// Trait for aggregating collections of services that can be relayed over an external bus.
    /// Do not implement this yourself - rather, rely on the `impl_odp_mctp_relay_handler` macro to implement this.
    ///
    pub trait RelayHandler {
        /// The type that uniquely identifies individual services. Generally expected to be a C-style enum.
        type ServiceIdType: Into<u8> + TryFrom<u8> + Copy;

        /// The header type used by request and result enums
        type HeaderType: mctp_rs::MctpMessageHeaderTrait + RelayHeader<Self::ServiceIdType>;

        /// An enum over all possible request types
        type RequestEnumType: for<'buf> mctp_rs::MctpMessageTrait<'buf, Header = Self::HeaderType>;

        /// An enum over all possible result types
        type ResultEnumType: for<'buf> mctp_rs::MctpMessageTrait<'buf, Header = Self::HeaderType>
            + RelayResponse<Self::ServiceIdType, Self::HeaderType>;

        /// Process the provided request and yield a result.
        fn process_request<'a>(
            &'a self,
            message: Self::RequestEnumType,
        ) -> impl core::future::Future<Output = Self::ResultEnumType> + 'a;

        /// Wait for a notification from any service and return the associated service notification ID.
        fn wait_for_notification<'a>(&'a mut self) -> impl core::future::Future<Output = u8> + 'a;
    }

    /// This macro generates a relay type over a collection of message types, which can be used by a relay service to
    /// receive messages over the wire and translate them into calls to a particular service on the EC.
    ///
    /// This is the recommended way to implement a relay handler - you should not implement the RelayHandler trait yourself.
    ///
    /// This macro will emit a type with the name you specify that is generic over a lifetime for the hardware (probably 'static in production code),
    /// implements the `RelayHandler` trait, and has a single constructor method `new` that takes as arguments references to the service handler
    /// types that you specify that have the 'hardware lifetime'.
    ///
    /// The macro takes the following inputs once:
    ///   relay_type_name: The name of the relay type to generate. This is arbitrary. The macro will emit a type with this name.
    ///
    /// Followed by a list of any number of service entries, which are specified by the following inputs:
    ///   service_name:            A name to assign to generated identifiers associated with the service, e.g. "Battery".
    ///                            This can be arbitrary.
    ///   service_id:              A unique u8 that addresses that service on the EC.
    ///   service_notification_id: A unique u8 identifying notifications from this service, distinct from service_id.
    ///   service_handler_type:    A type that implements the RelayServiceHandler trait, which will be used to process messages
    ///                            for this service.
    ///
    /// Example usage:
    ///
    /// ```ignore
    ///
    ///     impl_odp_mctp_relay_handler!(
    ///         MyRelayHanderType;
    ///         Battery,   0x9, 0, battery_service::Service<'static>;
    ///         TimeAlarm, 0xB, 1, time_alarm_service::Service<'static>;
    ///     );
    ///
    ///     let relay_handler = MyRelayHandlerType::new(battery_service_instance, time_alarm_service_instance);
    ///
    ///     // Then, pass relay_handler to your relay service (e.g. eSPI service), which should be generic over an `impl RelayHandler`.
    ///
    /// ```
    ///
    #[macro_export]
    macro_rules! impl_odp_mctp_relay_handler {
        (
            $relay_type_name:ident;
            $(
                $service_name:ident,
                $service_id:expr,
                $service_notification_id:expr,
                $service_handler_type:ty;
            )+
        ) => {
            $crate::_macro_internal::paste::paste! {
                mod [< _odp_impl_ $relay_type_name:snake >] {
                    use $crate::_macro_internal::bitfield::bitfield;
                    use core::convert::Infallible;
                    use $crate::_macro_internal::mctp_rs::smbus_espi::SmbusEspiMedium;
                    use $crate::_macro_internal::mctp_rs::{MctpMedium, MctpMessageHeaderTrait, MctpMessageTrait, MctpPacketError, MctpPacketResult};
                    use $crate::relay::{SerializableMessage, SerializableResult};
                    use $crate::relay::mctp::RelayServiceHandler;

                    #[derive(Debug, PartialEq, Eq, Clone, Copy)]
                    #[repr(u8)]
                    pub enum OdpService {
                        $(
                            $service_name = $service_id,
                        )+
                    }

                    impl From<OdpService> for u8 {
                        fn from(val: OdpService) -> u8 {
                            val as u8
                        }
                    }

                    impl TryFrom<u8> for OdpService {
                        type Error = u8;
                        fn try_from(value: u8) -> Result<Self, Self::Error> {
                            match value {
                                $(
                                    $service_id => Ok(OdpService::$service_name),
                                )+
                                other => Err(other),
                            }
                        }
                    }

                    pub enum HostRequest {
                        $(
                            $service_name(<$service_handler_type as $crate::relay::mctp::RelayServiceHandlerTypes>::RequestType),
                        )+
                    }

                    impl MctpMessageTrait<'_> for HostRequest {
                        type Header = OdpHeader;
                        const MESSAGE_TYPE: u8 = 0x7D; // ODP message type

                        fn serialize<M: MctpMedium>(self, buffer: &mut [u8]) -> MctpPacketResult<usize, M> {
                            match self {
                                $(
                                    HostRequest::$service_name(request) => SerializableMessage::serialize(request, buffer)
                                        .map_err(|_| MctpPacketError::SerializeError(concat!("Failed to serialize ", stringify!($service_name), " request"))),
                                )+
                            }
                        }

                        fn deserialize<M: MctpMedium>(header: &Self::Header, buffer: &'_ [u8]) -> MctpPacketResult<Self, M> {
                            Ok(match header.service {
                                $(
                                    OdpService::$service_name => Self::$service_name(
                                        <$service_handler_type as $crate::relay::mctp::RelayServiceHandlerTypes>::RequestType::deserialize(header.message_id, buffer)
                                            .map_err(|_| MctpPacketError::CommandParseError(concat!("Could not parse ", stringify!($service_name), " request")))?,
                                    ),
                                )+
                            })
                        }
                    }

                    bitfield! {
                        /// Wire format for ODP MCTP headers. Not user-facing - use OdpHeader instead.
                        #[derive(Copy, Clone, PartialEq, Eq)]
                        struct OdpHeaderWireFormat(u32);
                        impl Debug;
                        impl new;
                        /// If true, represents a request; otherwise, represents a result
                        is_request, set_is_request: 25;

                        /// The service ID that this message is related to
                        /// Note: Error checking is done when you access the field, not when you construct the OdpHeader. Take care when constructing a header.
                        u8, service_id, set_service_id: 23, 16;

                        /// On results, indicates if the result message is an error. Unused on requests.
                        is_error, set_is_error: 15;

                        /// The message type/discriminant
                        u16, message_id, set_message_id: 14, 0;
                    }

                    #[derive(Copy, Clone, PartialEq, Eq)]
                    pub enum OdpMessageType {
                        Request,
                        Result { is_error: bool },
                    }

                    #[derive(Copy, Clone, PartialEq, Eq)]
                    pub struct OdpHeader {
                        pub message_type: OdpMessageType,
                        pub service: OdpService,
                        pub message_id: u16,
                    }

                    impl From<OdpHeader> for OdpHeaderWireFormat {
                        fn from(src: OdpHeader) -> Self {
                            Self::new(
                                matches!(src.message_type, OdpMessageType::Request),
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

                    impl $crate::relay::mctp::RelayHeader<OdpService> for OdpHeader {
                        fn get_service_id(&self) -> OdpService {
                            self.service
                        }
                    }

                    #[derive(Clone)]
                    pub enum HostResult {
                        $(
                            $service_name(<$service_handler_type as $crate::relay::mctp::RelayServiceHandlerTypes>::ResultType),
                        )+
                    }

                    impl $crate::relay::mctp::RelayResponse<OdpService, OdpHeader> for HostResult {
                        fn create_header(&self, service_id: &OdpService) -> OdpHeader {
                            match (self) {
                                $(
                                    (HostResult::$service_name(result)) => OdpHeader {
                                        message_type: OdpMessageType::Result { is_error: !result.is_ok() },
                                        service: *service_id,
                                        message_id: result.discriminant(),
                                    },
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
                                        .map_err(|_| MctpPacketError::SerializeError(concat!("Failed to serialize ", stringify!($service_name), " result"))),
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
                                                Ok(HostResult::$service_name(<$service_handler_type as $crate::relay::mctp::RelayServiceHandlerTypes>::ResultType::deserialize(is_error, header.message_id, buffer)
                                                    .map_err(|_| MctpPacketError::CommandParseError(concat!("Could not parse ", stringify!($service_name), " result")))?))
                                            }
                                        }
                                    },
                                )+
                            }
                        }
                    }


                    pub struct $relay_type_name<'hw> {
                        $(
                            [<$service_name:snake _handler>]: &'hw $service_handler_type,
                            [<$service_name:snake _subscriber>]: $crate::_macro_internal::embassy_sync::pubsub::DynSubscriber<'hw, <$service_handler_type as $crate::relay::mctp::RelayServiceHandlerTypes>::MessageType>,
                        )+
                    }

                    impl<'hw> $relay_type_name<'hw> {
                        pub fn new(
                            $(
                                [<$service_name:snake _handler>]: &'hw $service_handler_type,
                                [<$service_name:snake _subscriber>]: $crate::_macro_internal::embassy_sync::pubsub::DynSubscriber<'hw, <$service_handler_type as $crate::relay::mctp::RelayServiceHandlerTypes>::MessageType>,
                            )+
                        ) -> Self {
                            Self {
                                $(
                                    [<$service_name:snake _handler>],
                                    [<$service_name:snake _subscriber>],
                                )+
                            }
                        }
                    }

                    impl<'hw> $crate::relay::mctp::RelayHandler for $relay_type_name<'hw> {
                        type ServiceIdType = OdpService;
                        type HeaderType = OdpHeader;
                        type RequestEnumType = HostRequest;
                        type ResultEnumType = HostResult;

                        fn process_request<'a>(
                            &'a self,
                            message: HostRequest,
                        ) -> impl core::future::Future<Output = HostResult> + 'a {
                            async move {
                                match message {
                                    $(
                                        HostRequest::$service_name(request) => {
                                            let result = self.[<$service_name:snake _handler>].process_request(request).await;
                                            HostResult::$service_name(result)
                                        }
                                    )+
                                }
                            }
                        }

                        // This waits for any relayable service to publish a message, then it checks if the message is a notification.
                        // If it is, it returns associated notification ID as defined by the macro.
                        // The relay service can then use this ID to determine how to notify the host SoC.
                        fn wait_for_notification<'a>(&'a mut self) -> impl core::future::Future<Output = u8> + 'a {
                            async move {
                                loop {
                                    $(
                                        let mut [<$service_name:snake _fut>] = core::pin::pin!(
                                            self.[<$service_name:snake _subscriber>].next_message()
                                        );
                                    )+

                                    let result = core::future::poll_fn(|cx| {
                                        $(
                                            if let core::task::Poll::Ready(wait_result) = [<$service_name:snake _fut>].as_mut().poll(cx) {
                                                match wait_result {
                                                    $crate::_macro_internal::embassy_sync::pubsub::WaitResult::Message(msg) => {
                                                        if <$service_handler_type as $crate::relay::mctp::RelayServiceHandler>::is_notification(&msg) {
                                                            return core::task::Poll::Ready(Some($service_notification_id));
                                                        } else {
                                                            return core::task::Poll::Ready(None);
                                                        }
                                                    }
                                                    $crate::_macro_internal::embassy_sync::pubsub::WaitResult::Lagged(count) => {
                                                        // Revisit: This can only happen if other services use a `publish_immediate` on their channel, which can result in older messages getting discarded.
                                                        // We really don't want notifications potentially getting lost, so we could change `SinglePublisherChannel` to not allow immediate publishing,
                                                        // or we could just keep the burden on services so they have the flexibility to use `publish_immediate` if they want to at the risk of their own notifications being lost.
                                                        $crate::error!("[Relay] {} subscriber lagged by {} messages, notifications may have been lost", stringify!($service_name), count);
                                                        return core::task::Poll::Ready(None);
                                                    }
                                                }
                                            }
                                        )+
                                        core::task::Poll::Pending
                                    }).await;

                                    if let Some(id) = result {
                                        return id;
                                    }
                                }
                            }
                        }
                    }
                } // end mod __odp_impl

                // Allows this generated relay type to be publicly re-exported
                pub use [< _odp_impl_ $relay_type_name:snake >]::$relay_type_name;

            } // end paste!
        }; // end macro arm
    } // end macro

    pub use impl_odp_mctp_relay_handler;
}
