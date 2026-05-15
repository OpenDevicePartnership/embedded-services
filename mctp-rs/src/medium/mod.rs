use crate::{
    buffer_encoding::{BufferEncoding, EncodingDecoder, EncodingEncoder},
    error::MctpPacketResult,
};

pub mod smbus_espi;
mod util;

#[cfg(feature = "serial")]
pub mod serial;

pub trait MctpMedium: Sized {
    /// the medium specific header and trailer for the packet
    type Frame: MctpMediumFrame<Self>;

    /// the error type for deserialization of the medium specific header
    type Error: core::fmt::Debug + Copy + Clone + PartialEq + Eq;

    // the type used for the data needed to send a reply to a request
    type ReplyContext: core::fmt::Debug + Copy + Clone + PartialEq + Eq;

    /// the byte-stuffing transform used by this medium when (de)serializing
    /// wire bytes. Stateless — see [`BufferEncoding`](crate::BufferEncoding).
    /// Most media use [`PassthroughEncoding`](crate::PassthroughEncoding)
    /// (no transform); media that need byte-stuffing (e.g., DSP0253 serial)
    /// supply their own impl.
    type Encoding: BufferEncoding;

    /// the maximum transmission unit for the medium
    fn max_message_body_size(&self) -> usize;

    /// Deserialize a packet into the medium-specific header (frame) and an
    /// [`EncodingDecoder`] that wraps the inner stuffed-region bytes.
    /// Higher layers (e.g., `parse_transport_header`, the payload copy
    /// loop in `MctpPacketContext`) read decoded bytes through the
    /// returned decoder and physically cannot bypass the medium's
    /// encoding by slicing the underlying buffer directly.
    fn deserialize<'buf>(
        &self,
        packet: &'buf [u8],
    ) -> MctpPacketResult<(Self::Frame, EncodingDecoder<'buf, Self::Encoding>), Self>;

    /// Serialize a packet by allowing the caller's `message_writer`
    /// closure to write decoded bytes into the medium's stuffed region
    /// through an [`EncodingEncoder`]. The medium owns its outer framing
    /// (e.g., SMBus header + PEC, DSP0253 start/end flags + FCS) and
    /// inspects the encoder's
    /// [`wire_position`](EncodingEncoder::wire_position) after the
    /// closure returns to size headers/trailers and compute checksums.
    fn serialize<'buf, F>(
        &self,
        reply_context: Self::ReplyContext,
        buffer: &'buf mut [u8],
        message_writer: F,
    ) -> MctpPacketResult<&'buf [u8], Self>
    where
        F: for<'a> FnOnce(&mut EncodingEncoder<'a, Self::Encoding>) -> MctpPacketResult<(), Self>;

    /// Returns `Ok(Some(len))` when `buf` contains a complete medium-framed
    /// packet starting at `buf[0]`, where `len` is the total byte count of
    /// that packet. Returns `Ok(None)` when `buf` has a partial frame
    /// (caller should read more bytes and retry). Returns `Err(...)` when
    /// `buf` is malformed.
    ///
    /// Used by generic consumers (e.g., `uart-service::Service<R, M>`)
    /// to assemble packets from byte streams without knowing medium
    /// framing details.
    fn frame_complete(&self, buf: &[u8]) -> MctpPacketResult<Option<usize>, Self>;
}

pub trait MctpMediumFrame<M: MctpMedium>: Clone + Copy {
    fn packet_size(&self) -> usize;
    fn reply_context(&self) -> M::ReplyContext;
}
