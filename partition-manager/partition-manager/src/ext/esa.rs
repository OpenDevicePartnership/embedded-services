//! Embedded Storage Async

use crate::{Error, Partition, PartitionGuard, RO, RW};
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
        let mut guard = self.lock().await;
        guard.read(offset, bytes).await
    }

    fn capacity(&self) -> usize {
        self.size as usize
    }
}

impl<F: ReadNorFlash, M: RawMutex> ReadNorFlash for Partition<'_, F, RW, M> {
    const READ_SIZE: usize = F::READ_SIZE;

    async fn read(&mut self, offset: u32, bytes: &mut [u8]) -> Result<(), Self::Error> {
        let mut guard = self.lock().await;
        guard.read(offset, bytes).await
    }

    fn capacity(&self) -> usize {
        self.size as usize
    }
}

impl<F: NorFlash, M: RawMutex> NorFlash for Partition<'_, F, RW, M> {
    const WRITE_SIZE: usize = F::WRITE_SIZE;
    const ERASE_SIZE: usize = F::ERASE_SIZE;

    async fn erase(&mut self, from: u32, to: u32) -> Result<(), Self::Error> {
        let mut guard = self.lock().await;
        guard.erase(from, to).await
    }

    async fn write(&mut self, offset: u32, bytes: &[u8]) -> Result<(), Self::Error> {
        let mut guard = self.lock().await;
        guard.write(offset, bytes).await
    }
}

impl<F: MultiwriteNorFlash, M: RawMutex> MultiwriteNorFlash for Partition<'_, F, RW, M> {}

impl<F: ReadNorFlash, MARKER, M: RawMutex> ErrorType for PartitionGuard<'_, F, MARKER, M> {
    type Error = Error<F::Error>;
}

impl<F: ReadNorFlash, MARKER, M: RawMutex> ReadNorFlash for PartitionGuard<'_, F, MARKER, M> {
    const READ_SIZE: usize = F::READ_SIZE;

    async fn read(&mut self, offset: u32, bytes: &mut [u8]) -> Result<(), Self::Error> {
        if !self.within_bounds(offset, bytes.len()) {
            return Err(Error::OutOfBounds);
        }

        self.guard
            .read(offset.checked_add(self.offset).ok_or(Error::OutOfBounds)?, bytes)
            .await
            .map_err(Error::Inner)
    }

    fn capacity(&self) -> usize {
        self.size as usize
    }
}

impl<F: NorFlash, M: RawMutex> NorFlash for PartitionGuard<'_, F, RW, M> {
    const WRITE_SIZE: usize = F::WRITE_SIZE;
    const ERASE_SIZE: usize = F::ERASE_SIZE;

    async fn erase(&mut self, from: u32, to: u32) -> Result<(), Self::Error> {
        if !self.within_bounds(from, to.checked_sub(from).ok_or(Error::OutOfBounds)? as usize) {
            return Err(Error::OutOfBounds);
        }

        self.guard
            .erase(
                from.checked_add(self.offset).ok_or(Error::OutOfBounds)?,
                to.checked_add(self.offset).ok_or(Error::OutOfBounds)?,
            )
            .await
            .map_err(Error::Inner)
    }

    async fn write(&mut self, offset: u32, bytes: &[u8]) -> Result<(), Self::Error> {
        if !self.within_bounds(offset, bytes.len()) {
            return Err(Error::OutOfBounds);
        }

        self.guard
            .write(offset.checked_add(self.offset).ok_or(Error::OutOfBounds)?, bytes)
            .await
            .map_err(Error::Inner)
    }
}

impl<F: MultiwriteNorFlash, M: RawMutex> MultiwriteNorFlash for PartitionGuard<'_, F, RW, M> {}
