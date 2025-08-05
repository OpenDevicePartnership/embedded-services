#![cfg_attr(not(test), no_std)]
#![doc = include_str!(concat!("../", env!("CARGO_PKG_README")))]

#[cfg(feature = "macros")]
pub use partition_manager_macros as macros;

use core::{fmt::Debug, marker::PhantomData};
use embassy_sync::{
    blocking_mutex::raw::{NoopRawMutex, RawMutex},
    mutex::Mutex,
};

mod ext;

#[cfg(test)]
mod test;

/// Manager for the partitions for a storage device.
///
/// Manages concurrent device access and ties lifetime to partitions.
pub struct PartitionManager<F, M: RawMutex = NoopRawMutex> {
    storage: Mutex<M, F>,
}

/// Partition of a disk.
///
/// If the underlying disk implements [embedded_storage_async::nor_flash::NorFlash] or [block_device_driver::BlockDevice], this partition will too.
/// Requires the features `esa` and/or `bdd` to be enabled for this crate.
#[allow(unused)]
pub struct Partition<'a, F, MARKER, M: RawMutex = NoopRawMutex> {
    storage: &'a Mutex<M, F>,
    offset: u32,
    size: u32,
    _marker: PhantomData<MARKER>,
}

impl<'a, F, MARKER, M: RawMutex> Partition<'a, F, MARKER, M> {
    pub const fn new(storage: &'a Mutex<M, F>, offset: u32, size: u32) -> Self {
        Self {
            storage,
            offset,
            size,
            _marker: PhantomData,
        }
    }
}

impl<F, M: RawMutex> Partition<'_, F, RW, M> {
    /// Temporarily convert a reference to a writable partition into a read-only partition.
    pub const fn readonly(&mut self) -> Partition<'_, F, RO, M> {
        Partition {
            storage: self.storage,
            offset: self.offset,
            size: self.size,
            _marker: PhantomData,
        }
    }
}

/// A partition configuration definition.
///
/// Using [PartitionManager::map] this definition can be turned into a concrete [PartitionMap].
pub trait PartitionConfig {
    type Map<'a, F, M: RawMutex>: PartitionMap
    where
        F: 'a,
        M: 'a;

    /// Instantiate partitions with a reference to an underlying storage.
    ///
    /// Typically end-users do not call this method directly, and instead use [PartitionManager::map].
    fn map<F, M: RawMutex>(self, storage: &Mutex<M, F>) -> Self::Map<'_, F, M>;
}

/// A concrete partition map.
pub trait PartitionMap {}

impl<F, M: RawMutex> PartitionManager<F, M> {
    /// Wrap a disk such that it can be concurrently accessed.
    pub const fn new(storage: F) -> Self {
        Self {
            storage: Mutex::new(storage),
        }
    }

    /// Map a disk to multiple partitions given a partition configuration definition.
    pub fn map<C: PartitionConfig>(&mut self, config: C) -> C::Map<'_, F, M> {
        config.map(&self.storage)
    }
}

impl<F, MARKER, M: RawMutex> Partition<'_, F, MARKER, M> {
    /// Checks whether an address range lies within the partition.
    #[allow(unused)]
    const fn within_bounds(&self, offset: u32, size: u32) -> bool {
        if let Some(end) = offset.checked_add(size) {
            end <= self.size
        } else {
            false
        }
    }
}

/// Marker type for read-only partitions.
pub struct RO;

/// Marker type for read/write partitions.
pub struct RW;

/// An error that can be returned on operations for partitions.
#[derive(Debug, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Error<E> {
    /// Operation went out of bounds of the partition.
    OutOfBounds,
    /// Operation is not aligned with the device alignment requirements.
    NotAligned,
    /// Tried to perform an Write or Erase operation on a read-only partition.
    ReadOnly,
    /// Underlying device returned an error.
    Inner(E),
}

impl<E> From<E> for Error<E> {
    fn from(value: E) -> Self {
        Error::Inner(value)
    }
}
