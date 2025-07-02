//! Embedded Storage Async

use crate::{Error, Partition, RO, RW};
use core::fmt::Debug;
use embassy_sync::blocking_mutex::raw::RawMutex;
use embedded_storage_async::nor_flash::{
    ErrorType, MultiwriteNorFlash, NorFlash, NorFlashError, NorFlashErrorKind, ReadNorFlash,
};

impl<E: NorFlashError + Debug> NorFlashError for Error<E> {
    fn kind(&self) -> NorFlashErrorKind {
        match self {
            Error::OutOfBounds => NorFlashErrorKind::OutOfBounds,
            Error::NotAligned => NorFlashErrorKind::NotAligned,
            Error::ReadOnly => NorFlashErrorKind::Other, // Note: actually unreachable, only thrown by other impls.
            Error::Inner(e) => e.kind(),
        }
    }
}

impl<F: ReadNorFlash, MARKER, M: RawMutex> ErrorType for Partition<'_, F, MARKER, M> {
    type Error = Error<F::Error>;
}

impl<F: ReadNorFlash, M: RawMutex> ReadNorFlash for Partition<'_, F, RO, M> {
    const READ_SIZE: usize = F::READ_SIZE;

    async fn read(&mut self, offset: u32, bytes: &mut [u8]) -> Result<(), Self::Error> {
        if !self.within_bounds(offset, bytes.len() as u32) {
            return Err(Error::OutOfBounds);
        }

        let mut storage = self.storage.lock().await;
        Ok(storage.read(offset + self.offset, bytes).await?)
    }

    fn capacity(&self) -> usize {
        self.size as usize
    }
}

impl<F: ReadNorFlash, M: RawMutex> ReadNorFlash for Partition<'_, F, RW, M> {
    const READ_SIZE: usize = F::READ_SIZE;

    async fn read(&mut self, offset: u32, bytes: &mut [u8]) -> Result<(), Self::Error> {
        self.readonly().read(offset, bytes).await
    }

    fn capacity(&self) -> usize {
        self.size as usize
    }
}

impl<F: NorFlash, M: RawMutex> NorFlash for Partition<'_, F, RW, M> {
    const WRITE_SIZE: usize = F::WRITE_SIZE;
    const ERASE_SIZE: usize = F::ERASE_SIZE;

    async fn erase(&mut self, from: u32, to: u32) -> Result<(), Self::Error> {
        if !self.within_bounds(from, to.saturating_sub(from)) {
            return Err(Error::OutOfBounds);
        }

        let mut storage = self.storage.lock().await;
        Ok(storage.erase(from + self.offset, to + self.offset).await?)
    }

    async fn write(&mut self, offset: u32, bytes: &[u8]) -> Result<(), Self::Error> {
        if !self.within_bounds(offset, bytes.len() as u32) {
            return Err(Error::OutOfBounds);
        }

        let mut storage = self.storage.lock().await;
        Ok(storage.write(offset + self.offset, bytes).await?)
    }
}

impl<F: MultiwriteNorFlash, M: RawMutex> MultiwriteNorFlash for Partition<'_, F, RW, M> {}
