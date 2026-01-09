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

// Prevent other types from implementing SerializableResponse - they should instead use SerializableMessage on a Response type and an Error type
#[doc(hidden)]
mod private {
    pub trait Sealed {}

    impl<T, E> Sealed for Result<T, E> {}
}

/// Responses are of type Result<T, E> where T and E both implement SerializableMessage
pub trait SerializableResponse: private::Sealed + Sized {
    /// The type of the response when the operation being responsed to succeeded
    type SuccessType: SerializableMessage;

    /// The type of the response when the operation being responsed to failed
    type ErrorType: SerializableMessage;

    /// Returns true if the response represents a successful operation, false otherwise
    fn is_ok(&self) -> bool;

    /// Returns a unique discriminant that can be used to deserialize the specific type of response.
    /// Discriminants can be reused for success and error messages.
    fn discriminant(&self) -> u16;

    /// Writes the response into the provided buffer.
    /// On success, returns the number of bytes written
    fn serialize(self, buffer: &mut [u8]) -> Result<usize, MessageSerializationError>;

    /// Attempts to deserialize the response from the provided buffer.
    fn deserialize(is_error: bool, discriminant: u16, buffer: &[u8]) -> Result<Self, MessageSerializationError>;
}

impl<T, E> SerializableResponse for Result<T, E>
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
