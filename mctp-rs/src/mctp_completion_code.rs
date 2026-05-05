use bit_register::{NumBytes, TryFromBits, TryIntoBits};

#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum MctpCompletionCode {
    #[default]
    Success,
    Error,
    ErrorInvalidData,
    ErrorInvalidLength,
    ErrorNotReady,
    ErrorUnsupportedCmd,
    CommandSpecific(u8), // 0x80-0xFF are command specific
}

impl From<MctpCompletionCode> for u8 {
    fn from(value: MctpCompletionCode) -> Self {
        match value {
            MctpCompletionCode::Success => 0x00,
            MctpCompletionCode::Error => 0x01,
            MctpCompletionCode::ErrorInvalidData => 0x02,
            MctpCompletionCode::ErrorInvalidLength => 0x03,
            MctpCompletionCode::ErrorNotReady => 0x04,
            MctpCompletionCode::ErrorUnsupportedCmd => 0x05,
            MctpCompletionCode::CommandSpecific(code) => code,
        }
    }
}
impl TryFrom<u8> for MctpCompletionCode {
    type Error = &'static str;
    fn try_from(value: u8) -> Result<Self, &'static str> {
        Ok(match value {
            0x00 => MctpCompletionCode::Success,
            0x01 => MctpCompletionCode::Error,
            0x02 => MctpCompletionCode::ErrorInvalidData,
            0x03 => MctpCompletionCode::ErrorInvalidLength,
            0x04 => MctpCompletionCode::ErrorNotReady,
            0x05 => MctpCompletionCode::ErrorUnsupportedCmd,
            0x06..=0x7F => return Err("Invalid value for MCTP completion code - reserved range"),
            0x80..=0xFF => MctpCompletionCode::CommandSpecific(value),
        })
    }
}

impl TryFromBits<u32> for MctpCompletionCode {
    fn try_from_bits(bits: u32) -> Result<Self, &'static str> {
        if bits > 0xFF {
            return Err("Out of range value for MCTP completion code");
        }
        (bits as u8).try_into()
    }
}

impl TryIntoBits<u32> for MctpCompletionCode {
    fn try_into_bits(self) -> Result<u32, &'static str> {
        Ok(Into::<u8>::into(self) as u32)
    }
}

impl NumBytes for MctpCompletionCode {
    const NUM_BYTES: usize = 1;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_completion_code_reserved_range() {
        // Test that reserved range 0x06-0x7F is properly rejected
        for code in 0x06..=0x7F {
            let result = MctpCompletionCode::try_from(code);
            assert!(result.is_err(), "Code 0x{:02X} should be rejected", code);
            if let Err(msg) = result {
                assert!(msg.contains("reserved range"));
            }
        }

        // Test valid ranges still work
        assert_eq!(
            MctpCompletionCode::try_from(0x00).unwrap(),
            MctpCompletionCode::Success
        );
        assert_eq!(
            MctpCompletionCode::try_from(0x05).unwrap(),
            MctpCompletionCode::ErrorUnsupportedCmd
        );
        assert_eq!(
            MctpCompletionCode::try_from(0x80).unwrap(),
            MctpCompletionCode::CommandSpecific(0x80)
        );
        assert_eq!(
            MctpCompletionCode::try_from(0xFF).unwrap(),
            MctpCompletionCode::CommandSpecific(0xFF)
        );
    }
}
