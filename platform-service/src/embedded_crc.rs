use crc::Algorithm;

pub struct EmbeddedCrc<W: crc::Width> {
    algorithm: &'static Algorithm<W>,
    current_crc: Option<W>,
}

#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[derive(Clone, Copy, Debug, Default)]
pub enum EmbeddedCrcError {
    #[default]
    CrcErrorUnknown,
    CrcErrorWidth,
    CrcErrorPolynomial,
    CrcErrorXorOut,
    CrcErrorMutexGet,
}

impl EmbeddedCrc<u32> {
    pub const fn new(algorithm: &'static Algorithm<u32>) -> Self {
        Self {
            algorithm,
            current_crc: None,
        }
    }

    pub async fn calculate(&mut self, bytes: &[u8]) -> Result<u32, EmbeddedCrcError> {
        // Set starting value for CRC calculation
        let initial = if self.current_crc.is_none() {
            // Use the algorithm initial value if no CRC has begun calculation
            self.algorithm.init
        } else {
            // For split calculations, undo the algorithm's output adjustments
            self.un_finalize(self.current_crc.unwrap())
        };

        match crate::crc_calculate_u32(initial, self.algorithm, bytes).await {
            Ok(crc) => {
                self.current_crc = Some(crc);
                Ok(crc)
            }
            Err(e) => {
                // Errored CRC calculations do not reset the stored CRC
                // Users can attempt the calculation from the same point
                Err(e)
            }
        }
    }

    pub fn read_crc(&self) -> u32 {
        self.current_crc.unwrap_or(self.algorithm.init)
    }

    // Reverses the digest finalize operation to use as another CRC input
    fn un_finalize(&self, crc: u32) -> u32 {
        let mut out: u32 = crc ^ self.algorithm.xorout;
        if self.algorithm.refout {
            out <<= 32u8 - self.algorithm.width;
        }
        // Actual finalize uses the condition 'refin ^ refout',
        // However, the refin field reverses bits on the input as well, so this does not need be considered here
        if self.algorithm.refout {
            out = out.reverse_bits();
        }
        out
    }
}

impl EmbeddedCrc<u16> {
    pub const fn new(algorithm: &'static Algorithm<u16>) -> Self {
        Self {
            algorithm,
            current_crc: None,
        }
    }

    pub async fn calculate(&mut self, bytes: &[u8]) -> Result<u16, EmbeddedCrcError> {
        // Set starting value for CRC calculation
        let initial = if self.current_crc.is_none() {
            // Use the algorithm initial value if no CRC has begun calculation
            self.algorithm.init
        } else {
            // For split calculations, undo the algorithm's output adjustments
            self.un_finalize(self.current_crc.unwrap())
        };

        match crate::crc_calculate_u16(initial, self.algorithm, bytes).await {
            Ok(crc) => {
                self.current_crc = Some(crc);
                Ok(crc)
            }
            Err(e) => {
                // Errored CRC calculations do not reset the stored CRC
                // Users can attempt the calculation from the same point
                Err(e)
            }
        }
    }

    pub fn read_crc(&self) -> u16 {
        self.current_crc.unwrap_or(self.algorithm.init)
    }

    // Reverses the digest finalize operation to use as another CRC input
    fn un_finalize(&self, crc: u16) -> u16 {
        let mut out: u16 = crc ^ self.algorithm.xorout;
        if self.algorithm.refout {
            out <<= 16u8 - self.algorithm.width;
        }
        // Actual finalize uses the condition 'refin ^ refout',
        // However, the refin field reverses bits on the input as well, so this does not need be considered here
        if self.algorithm.refout {
            out = out.reverse_bits();
        }
        out
    }
}
