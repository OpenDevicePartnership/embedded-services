# Partition manager #![no_std]

> Statically checked storage device mapping

## Architecture

Partition manager consists of these parts:

- `partition-manager`: The main crate you as device user should include and use in your project.
  It provides the implementation to split a storage device up into partitions and how to use these partitions as `embedded-storage-async` or `block-device-driver`.
  By default it also reexports the macros to generate a partition definition and mapping.
- `partition-manager-generation`: The generation crate takes the textual partition configuration and generates the Rust implementation for it. It also checks the textual partition configuration.
- `partition-manager-macros`: A small proc macro frontend to invoke the generation.

## Guide

Given a partition map, you can instantiate *variants* of this partition map given your application role. Say you have a bootloader stage, and an application stage on your device. In this case some partitions may be inaccessible or read-only, depending on which stage the firmware is running in.

To start, assuming you have enabled the `default` features or at least `macros` and `toml`, you can define your partition as a `json` or `toml` file:

```toml
variants = ["bootloader", "application"]

[disk]
size = 0x4000
alignment = 0x0100

[partitions]
factory = { offset = 0x0000, size = 0x0100, access = { any = "ro" } }
settings = { offset = 0x0100, size = 0x0200, access = { any = "ro", bootloader = "rw" } }
slot_a = { offset = 0x1000, size = 0x1000 }
slot_b = { offset = 0x2000, size = 0x1000 }
```

In your code you can state:

```rust,ignore
partition_manager_macros::create_partition_map!(
    name: StorageConfig,
    map_name: StorageMap,
    variant: "bootloader",
    manifest: "partitions.toml"
);
```

Which will define the structs `StorageConfig` and `StorageMap`. Given any `disk` you can instantiate the `StorageMap` as follows:

```rust,ignore
let mut pm: PartitionManager<_> = PartitionManager::new(&mut disk);
let StorageMap {
    mut factory,
    mut settings,
    mut slot_a,
    mut slot_b,
} = pm.map(TestConfig);
```

If for the variant a disk is not accessible, it will not be defined in the `StorageMap` struct. Markers `RO` and `RW` are set depending on the access rules and the requested variant.

### Embedded Storage Async
For these partitions the [`NorFlash`](https://docs.rs/embedded-storage-async/0.4.1/embedded_storage_async/nor_flash/trait.NorFlash.html) trait is implemented if all of the following are true: 
* the feature `esa` is enabled
* `disk` implements the trait
* the partition is `RW`

Similarly if the partition is `RO` only the trait [`ReadNorFlash`](https://docs.rs/embedded-storage-async/0.4.1/embedded_storage_async/nor_flash/trait.ReadNorFlash.html) is implemented.

### Block Device Driver
For these partitions the [`BlockDevice`](https://docs.rs/block-device-driver/0.2.0/block_device_driver/trait.BlockDevice.html) trait is implemented if all of the following are true: 
* the feature `bdd` is enabled
* `disk` implements the trait

For partitions that are `RO` write operations for this trait result in a runtime error (not panic) `ReadOnly`.

### Mutex
Access to the underlying storage device is managed by an [`Mutex`](https://docs.embassy.dev/embassy-sync/git/default/mutex/struct.Mutex.html). If you want the partitions to be `Sync` you can specify another [`RawMutex`](https://docs.embassy.dev/embassy-sync/git/default/blocking_mutex/raw/trait.RawMutex.html) when constructing the `PartitionManager`.

### Checks
Only when using the macro, the partition map is checked whether the partitions:
* are within the bounds of the storage device.
* align with the alignment requirements of the storage device.
* do not overlap between eachother.

**Note:** the associated constants for the various Embedded Storage Async traits and Block Device Driver traits are not checked to the alignment specified in the macro manifests.
