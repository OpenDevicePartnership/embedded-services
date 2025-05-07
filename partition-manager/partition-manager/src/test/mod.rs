mod mock;

#[cfg(feature = "bdd")]
mod bdd;
#[cfg(feature = "esa")]
mod esa;
#[cfg(feature = "macros")]
mod macros;

use core::marker::PhantomData;

use crate::{Partition, PartitionConfig, PartitionMap, RW};
use embassy_sync::blocking_mutex::raw::{NoopRawMutex, RawMutex};

#[allow(unused)]
struct TestMap<'a, F, M: RawMutex = NoopRawMutex> {
    pub factory: Partition<'a, F, RW, M>,
    pub settings: Partition<'a, F, RW, M>,
    pub slot_a: Partition<'a, F, RW, M>,
    pub slot_b: Partition<'a, F, RW, M>,
}

impl<'a, F, M: RawMutex> PartitionMap for TestMap<'a, F, M> {}

struct TestConfig;

impl PartitionConfig for TestConfig {
    type Map<'a, F, M: embassy_sync::blocking_mutex::raw::RawMutex>
        = TestMap<'a, F, M>
    where
        F: 'a,
        M: 'a;

    fn map<F, M: embassy_sync::blocking_mutex::raw::RawMutex>(
        self,
        storage: &embassy_sync::mutex::Mutex<M, F>,
    ) -> Self::Map<'_, F, M> {
        TestMap {
            factory: Partition {
                storage,
                offset: 0x0000,
                size: 0x0100,
                _marker: PhantomData,
            },
            settings: Partition {
                storage,
                offset: 0x0100,
                size: 0x0200,
                _marker: PhantomData,
            },
            slot_a: Partition {
                storage,
                offset: 0x1000,
                size: 0x1000,
                _marker: PhantomData,
            },
            slot_b: Partition {
                storage,
                offset: 0x2000,
                size: 0x1000,
                _marker: PhantomData,
            },
        }
    }
}
