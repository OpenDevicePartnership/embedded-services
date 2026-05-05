use crate::error::MctpPacketResult;

pub mod smbus_espi;
mod util;

pub trait MctpMedium: Sized {
    /// the medium specific header and trailer for the packet
    type Frame: MctpMediumFrame<Self>;

    /// the error type for deserialization of the medium specific header
    type Error: core::fmt::Debug + Copy + Clone + PartialEq + Eq;

    // the type used for the data needed to send a reply to a request
    type ReplyContext: core::fmt::Debug + Copy + Clone + PartialEq + Eq;

    /// the maximum transmission unit for the medium
    fn max_message_body_size(&self) -> usize;

    /// deserialize the packet into the medium specific header and remainder of the packet -
    /// this includes the mctp transport header, and mctp packet payload
    fn deserialize<'buf>(
        &self,
        packet: &'buf [u8],
    ) -> MctpPacketResult<(Self::Frame, &'buf [u8]), Self>;

    /// serialize the packet into the medium specific header and the payload
    fn serialize<'buf, F>(
        &self,
        reply_context: Self::ReplyContext,
        buffer: &'buf mut [u8],
        message_writer: F,
    ) -> MctpPacketResult<&'buf [u8], Self>
    where
        F: for<'a> FnOnce(&'a mut [u8]) -> MctpPacketResult<usize, Self>;
}

pub trait MctpMediumFrame<M: MctpMedium>: Clone + Copy {
    fn packet_size(&self) -> usize;
    fn reply_context(&self) -> M::ReplyContext;
}
