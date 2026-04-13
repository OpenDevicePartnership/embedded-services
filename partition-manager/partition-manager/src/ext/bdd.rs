//! Block Device Driver

use aligned::Aligned;
use block_device_driver::BlockDevice;
use embassy_sync::blocking_mutex::raw::RawMutex;

use crate::{Error, Partition, PartitionGuard, RO, RW};

impl<F, MARKER, M: RawMutex> Partition<'_, F, MARKER, M> {
    /// Returns the block number on the parent storage medium, given a block size.
    ///
    /// Will not be able to return a value of the partition is not aligned to a single block.
    const fn start_block(&self, block_size: u32) -> Option<u32> {
        if self.offset % block_size != 0 {
            None
        } else {
            Some(self.offset / block_size)
        }
    }

    /// Check if data access for a block address and set of blocks lies completely within this partition.
    const fn check_access<const SIZE: usize>(
        &self,
        block_address: u32,
        data: &[Aligned<F::Align, [u8; SIZE]>],
    ) -> Result<(), Error<F::Error>>
    where
        F: BlockDevice<SIZE>,
    {
        const { assert!(SIZE <= u32::MAX as usize) };
        let Some(offset) = block_address.checked_mul(SIZE as u32) else {
            return Err(Error::OutOfBounds);
        };
        if data.len() > u32::MAX as usize {
            return Err(Error::OutOfBounds);
        }
        let Some(size) = (data.len() as u32).checked_mul(SIZE as u32) else {
            return Err(Error::OutOfBounds);
        };
        if !self.within_bounds(offset, size) {
            Err(Error::OutOfBounds)
        } else {
            Ok(())
        }
    }
}

impl<F, MARKER, M: RawMutex> PartitionGuard<'_, F, MARKER, M> {
    const fn start_block(&self, block_size: u32) -> Option<u32> {
        if self.offset % block_size != 0 {
            None
        } else {
            Some(self.offset / block_size)
        }
    }

    const fn check_access<const SIZE: usize>(
        &self,
        block_address: u32,
        data: &[Aligned<F::Align, [u8; SIZE]>],
    ) -> Result<(), Error<F::Error>>
    where
        F: BlockDevice<SIZE>,
    {
        const { assert!(SIZE <= u32::MAX as usize) };
        let Some(offset) = block_address.checked_mul(SIZE as u32) else {
            return Err(Error::OutOfBounds);
        };
        if data.len() > u32::MAX as usize {
            return Err(Error::OutOfBounds);
        }
        let Some(size) = (data.len() as u32).checked_mul(SIZE as u32) else {
            return Err(Error::OutOfBounds);
        };
        if !self.within_bounds(offset, size) {
            Err(Error::OutOfBounds)
        } else {
            Ok(())
        }
    }
}

impl<const SIZE: usize, F: BlockDevice<SIZE>, M: RawMutex> BlockDevice<SIZE> for Partition<'_, F, RO, M> {
    type Error = Error<F::Error>;
    type Align = F::Align;

    async fn read(
        &mut self,
        block_address: u32,
        data: &mut [Aligned<Self::Align, [u8; SIZE]>],
    ) -> Result<(), Self::Error> {
        self.check_access(block_address, data)?;
        let start_block = self.start_block(SIZE as u32).ok_or(Error::NotAligned)?;

        let mut storage = self.storage.lock().await;
        Ok(storage
            .read(start_block.checked_add(block_address).ok_or(Error::OutOfBounds)?, data)
            .await?)
    }

    async fn write(
        &mut self,
        _block_address: u32,
        _data: &[Aligned<Self::Align, [u8; SIZE]>],
    ) -> Result<(), Self::Error> {
        Err(Error::ReadOnly)
    }

    async fn size(&mut self) -> Result<u64, Self::Error> {
        Ok(self.size as u64)
    }
}

impl<const SIZE: usize, F: BlockDevice<SIZE>, M: RawMutex> BlockDevice<SIZE> for Partition<'_, F, RW, M> {
    type Error = Error<F::Error>;
    type Align = F::Align;

    async fn read(
        &mut self,
        block_address: u32,
        data: &mut [Aligned<Self::Align, [u8; SIZE]>],
    ) -> Result<(), Self::Error> {
        self.readonly().read(block_address, data).await
    }

    async fn write(
        &mut self,
        block_address: u32,
        data: &[Aligned<Self::Align, [u8; SIZE]>],
    ) -> Result<(), Self::Error> {
        self.check_access(block_address, data)?;
        let start_block = self.start_block(SIZE as u32).ok_or(Error::NotAligned)?;

        let mut storage = self.storage.lock().await;
        Ok(storage
            .write(start_block.checked_add(block_address).ok_or(Error::OutOfBounds)?, data)
            .await?)
    }

    async fn size(&mut self) -> Result<u64, Self::Error> {
        Ok(self.size as u64)
    }
}

// PartitionGuard trait implementations
impl<const SIZE: usize, F: BlockDevice<SIZE>, M: RawMutex> BlockDevice<SIZE> for PartitionGuard<'_, F, RO, M> {
    type Error = Error<F::Error>;
    type Align = F::Align;

    async fn read(
        &mut self,
        block_address: u32,
        data: &mut [Aligned<Self::Align, [u8; SIZE]>],
    ) -> Result<(), Self::Error> {
        self.check_access(block_address, data)?;
        let start_block = self.start_block(SIZE as u32).ok_or(Error::NotAligned)?;

        Ok(self
            .guard
            .read(start_block.checked_add(block_address).ok_or(Error::OutOfBounds)?, data)
            .await?)
    }

    async fn write(
        &mut self,
        _block_address: u32,
        _data: &[Aligned<Self::Align, [u8; SIZE]>],
    ) -> Result<(), Self::Error> {
        Err(Error::ReadOnly)
    }

    async fn size(&mut self) -> Result<u64, Self::Error> {
        Ok(self.size as u64)
    }
}

impl<const SIZE: usize, F: BlockDevice<SIZE>, M: RawMutex> BlockDevice<SIZE> for PartitionGuard<'_, F, RW, M> {
    type Error = Error<F::Error>;
    type Align = F::Align;

    async fn read(
        &mut self,
        block_address: u32,
        data: &mut [Aligned<Self::Align, [u8; SIZE]>],
    ) -> Result<(), Self::Error> {
        self.check_access(block_address, data)?;
        let start_block = self.start_block(SIZE as u32).ok_or(Error::NotAligned)?;

        Ok(self
            .guard
            .read(start_block.checked_add(block_address).ok_or(Error::OutOfBounds)?, data)
            .await?)
    }

    async fn write(
        &mut self,
        block_address: u32,
        data: &[Aligned<Self::Align, [u8; SIZE]>],
    ) -> Result<(), Self::Error> {
        self.check_access(block_address, data)?;
        let start_block = self.start_block(SIZE as u32).ok_or(Error::NotAligned)?;

        Ok(self
            .guard
            .write(start_block.checked_add(block_address).ok_or(Error::OutOfBounds)?, data)
            .await?)
    }

    async fn size(&mut self) -> Result<u64, Self::Error> {
        Ok(self.size as u64)
    }
}
