use super::*;

pub struct VendorDefinedPci<'buf>(pub &'buf [u8]);
pub struct VendorDefinedPciHeader(pub u16);
const HEADER_LEN: usize = size_of::<u16>();

impl MctpMessageHeaderTrait for VendorDefinedPciHeader {
    fn serialize<M: MctpMedium>(self, buffer: &mut [u8]) -> MctpPacketResult<usize, M> {
        if buffer.len() < HEADER_LEN {
            return Err(crate::MctpPacketError::SerializeError(
                "buffer too small for vendor defined pci header",
            ));
        }
        buffer[..HEADER_LEN].copy_from_slice(&self.0.to_be_bytes());
        Ok(HEADER_LEN)
    }

    fn deserialize<M: MctpMedium>(buffer: &[u8]) -> MctpPacketResult<(Self, &[u8]), M> {
        if buffer.len() < HEADER_LEN {
            return Err(crate::MctpPacketError::HeaderParseError(
                "buffer too small for vendor defined pci header",
            ));
        }
        let header =
            VendorDefinedPciHeader(u16::from_be_bytes(buffer[..HEADER_LEN].try_into().unwrap()));
        Ok((header, &buffer[HEADER_LEN..]))
    }
}

impl<'buf> MctpMessageTrait<'buf> for VendorDefinedPci<'buf> {
    type Header = VendorDefinedPciHeader;
    const MESSAGE_TYPE: u8 = 0x7E;

    fn serialize<M: MctpMedium>(self, buffer: &mut [u8]) -> MctpPacketResult<usize, M> {
        let self_len = self.0.len();
        if buffer.len() < self_len {
            return Err(crate::MctpPacketError::SerializeError(
                "buffer too small for vendor defined pci message",
            ));
        }
        buffer[..self_len].copy_from_slice(self.0);
        Ok(self_len)
    }

    fn deserialize<M: MctpMedium>(
        _: &Self::Header,
        buffer: &'buf [u8],
    ) -> MctpPacketResult<VendorDefinedPci<'buf>, M> {
        let message = VendorDefinedPci(buffer);
        Ok(message)
    }
}
