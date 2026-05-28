//! Serialization/deserialization helpers for ODP relay request/response message types.

/// Error type for serializing/deserializing messages.
#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum MessageSerializationError {
    /// The message payload does not represent a valid message.
    InvalidPayload(&'static str),

    /// The message discriminant does not represent a known message type.
    UnknownMessageDiscriminant(u16),

    /// The provided buffer is too small to serialize the message.
    BufferTooSmall,

    /// Unspecified error.
    Other(&'static str),
}

/// Trait for serializing and deserializing messages.
pub trait SerializableMessage: Sized {
    /// Serialize the message into `buffer`, returning the number of bytes written.
    fn serialize(self, buffer: &mut [u8]) -> Result<usize, MessageSerializationError>;

    /// Discriminant used to identify the concrete message type on deserialize.
    fn discriminant(&self) -> u16;

    /// Deserialize a message of the type identified by `discriminant` from `buffer`.
    fn deserialize(discriminant: u16, buffer: &[u8]) -> Result<Self, MessageSerializationError>;
}

// Sealed: only `Result<T, E>` may implement `SerializableResult`. Custom response
// types should implement `SerializableMessage` on a Response and an Error type and
// use `Result<Response, Error>` for the result.
#[doc(hidden)]
mod private {
    pub trait Sealed {}

    impl<T, E> Sealed for Result<T, E> {}
}

/// Sealed trait implemented for `Result<T, E>` where both `T` and `E`
/// implement [`SerializableMessage`]. Used for response ("result") types.
pub trait SerializableResult: private::Sealed + Sized {
    /// The success arm's message type.
    type SuccessType: SerializableMessage;

    /// The error arm's message type.
    type ErrorType: SerializableMessage;

    /// `true` if this result represents a successful operation.
    fn is_ok(&self) -> bool;

    /// Discriminant of the inner success or error message. Success and
    /// error variants may reuse the same discriminant.
    fn discriminant(&self) -> u16;

    /// Serialize the inner message into `buffer`, returning the number of bytes written.
    fn serialize(self, buffer: &mut [u8]) -> Result<usize, MessageSerializationError>;

    /// Deserialize a result. `is_error` selects between success and error decoding.
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
