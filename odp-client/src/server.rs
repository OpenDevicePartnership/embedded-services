//! Server-side relay traits and the [`impl_odp_relay_handler`] macro.
//!
//! Provides traits that individual service handlers implement, plus the
//! macro that aggregates them into a single [`RelayHandler`] suitable for
//! plugging into a relay service (e.g. the eSPI service).

/// Companion-types trait for [`RelayServiceHandler`]: declares the
/// request and result types for a single service.
pub trait RelayServiceHandlerTypes {
    /// The request type this service handler processes.
    type RequestType: crate::serializable::SerializableMessage;

    /// The result type this service handler produces.
    type ResultType: crate::serializable::SerializableResult;
}

/// Trait for a service that can be relayed over an external bus (e.g.
/// battery service, thermal service, time-alarm service).
pub trait RelayServiceHandler: RelayServiceHandlerTypes {
    /// Process the provided request and yield a result.
    fn process_request<'a>(
        &'a self,
        request: Self::RequestType,
    ) -> impl core::future::Future<Output = Self::ResultType> + 'a;
}

// Traits below this point are intended for consumption by relay services (e.g. the eSPI service),
// not individual services that want their messages relayed. Implement these via the
// `impl_odp_relay_handler` macro rather than by hand.

/// Additional methods required on the relay header type. Implemented by
/// the [`impl_odp_relay_handler`] macro; do not implement directly.
#[doc(hidden)]
pub trait RelayHeader<ServiceIdType> {
    /// Return the ID of the service associated with the request.
    fn get_service_id(&self) -> ServiceIdType;
}

/// Additional methods required on the relay response type. Implemented
/// by the [`impl_odp_relay_handler`] macro; do not implement directly.
#[doc(hidden)]
pub trait RelayResponse<ServiceIdType, HeaderType> {
    /// Construct a header for this result given its service ID.
    fn create_header(&self, service_id: &ServiceIdType) -> HeaderType;
}

/// Aggregates a collection of services into a single relay surface.
/// Implemented by the [`impl_odp_relay_handler`] macro; do not implement
/// directly.
pub trait RelayHandler {
    /// The type that uniquely identifies individual services. Generally a C-style enum.
    type ServiceIdType: Into<u8> + TryFrom<u8> + Copy;

    /// The header type used by request and result enums.
    type HeaderType: mctp_rs::MctpMessageHeaderTrait + RelayHeader<Self::ServiceIdType>;

    /// An enum over all possible request types.
    type RequestEnumType: for<'buf> mctp_rs::MctpMessageTrait<'buf, Header = Self::HeaderType>;

    /// An enum over all possible result types.
    type ResultEnumType: for<'buf> mctp_rs::MctpMessageTrait<'buf, Header = Self::HeaderType>
        + RelayResponse<Self::ServiceIdType, Self::HeaderType>;

    /// Process the provided request and yield a result.
    fn process_request<'a>(
        &'a self,
        message: Self::RequestEnumType,
    ) -> impl core::future::Future<Output = Self::ResultEnumType> + 'a;
}

/// Generates a relay type that aggregates multiple service handlers into a
/// single [`RelayHandler`] suitable for use by a relay service (e.g. the
/// eSPI service). This is the recommended way to obtain a `RelayHandler`
/// — do not implement that trait by hand.
///
/// Inputs:
///   - `relay_type_name`: identifier for the generated struct
///   - For each service:
///     - `service_name`: identifier used to name fields and variants
///     - `service_id`: unique `u8` addressing the service on the EC
///     - `service_handler_type`: a type implementing [`RelayServiceHandler`]
///
/// The generated type exposes a `new` constructor taking one
/// `service_handler_type` argument per registered service.
///
/// # Example
///
/// ```ignore
/// impl_odp_relay_handler!(
///     MyRelayHandlerType;
///     Battery,   0x9, battery_service_relay::RelayHandler<battery_service::Service<'static>>;
///     TimeAlarm, 0xB, time_alarm_service_relay::RelayHandler<time_alarm_service::Service<'static>>;
/// );
///
/// let relay_handler = MyRelayHandlerType::new(battery_handler, time_alarm_handler);
/// // Pass relay_handler to a relay service that is generic over `impl RelayHandler`.
/// ```
#[macro_export]
macro_rules! impl_odp_relay_handler {
    (
        $relay_type_name:ident;
        $(
            $service_name:ident,
            $service_id:expr,
            $service_handler_type:ty;
        )+
    ) => {
        $crate::_macro_internal::paste::paste! {
            mod [< _odp_impl_ $relay_type_name:snake >] {
                use $crate::_macro_internal::bitfield::bitfield;
                use core::convert::Infallible;
                use $crate::_macro_internal::mctp_rs::smbus_espi::SmbusEspiMedium;
                use $crate::_macro_internal::mctp_rs::{MctpMedium, MctpMessageHeaderTrait, MctpMessageTrait, MctpPacketError, MctpPacketResult};
                use $crate::serializable::{SerializableMessage, SerializableResult};
                use $crate::server::RelayServiceHandler;

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
                        $service_name(<$service_handler_type as $crate::server::RelayServiceHandlerTypes>::RequestType),
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
                                    <$service_handler_type as $crate::server::RelayServiceHandlerTypes>::RequestType::deserialize(header.message_id, buffer)
                                        .map_err(|_| MctpPacketError::CommandParseError(concat!("Could not parse ", stringify!($service_name), " request")))?,
                                ),
                            )+
                        })
                    }
                }

                bitfield! {
                    /// Wire format for ODP headers. Not user-facing — use OdpHeader instead.
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

                impl $crate::server::RelayHeader<OdpService> for OdpHeader {
                    fn get_service_id(&self) -> OdpService {
                        self.service
                    }
                }

                #[derive(Clone)]
                pub enum HostResult {
                    $(
                        $service_name(<$service_handler_type as $crate::server::RelayServiceHandlerTypes>::ResultType),
                    )+
                }

                impl $crate::server::RelayResponse<OdpService, OdpHeader> for HostResult {
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
                                            Ok(HostResult::$service_name(<$service_handler_type as $crate::server::RelayServiceHandlerTypes>::ResultType::deserialize(is_error, header.message_id, buffer)
                                                .map_err(|_| MctpPacketError::CommandParseError(concat!("Could not parse ", stringify!($service_name), " result")))?))
                                        }
                                    }
                                },
                            )+
                        }
                    }
                }


                pub struct $relay_type_name {
                    $(
                        [<$service_name:snake _handler>]: $service_handler_type,
                    )+
                }

                impl $relay_type_name {
                    pub fn new(
                        $(
                            [<$service_name:snake _handler>]: $service_handler_type,
                        )+
                    ) -> Self {
                        Self {
                            $(
                                [<$service_name:snake _handler>],
                            )+
                        }
                    }
                }

                impl $crate::server::RelayHandler for $relay_type_name {
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
                }
            } // end mod __odp_impl

            // Allows this generated relay type to be publicly re-exported
            pub use [< _odp_impl_ $relay_type_name:snake >]::$relay_type_name;

        } // end paste!
    }; // end macro arm
} // end macro
