use crate::embedded_crc::EmbeddedCrcError;
use crc::Algorithm;

pub(crate) fn crc_calculate_u32(
    init: u32,
    algorithm: &'static Algorithm<u32>,
    bytes: &[u8],
) -> Result<u32, EmbeddedCrcError> {
    let crc = crc::Crc::<u32>::new(algorithm);
    let mut digest = crc.digest_with_initial(init);
    digest.update(bytes);
    Ok(digest.finalize())
}

pub(crate) fn crc_calculate_u16(
    init: u16,
    algorithm: &'static Algorithm<u16>,
    bytes: &[u8],
) -> Result<u16, EmbeddedCrcError> {
    let crc = crc::Crc::<u16>::new(algorithm);
    let mut digest = crc.digest_with_initial(init);
    digest.update(bytes);
    Ok(digest.finalize())
}
