use crate::{
    Error, PartitionManager,
    test::{
        TestConfig, TestMap,
        mock::{ActionErase, ActionRead, ActionWrite, MockDisk},
    },
};

#[test]
fn bdd() {
    embassy_futures::block_on(async {
        use block_device_driver::{slice_to_blocks, slice_to_blocks_mut};
        use std::collections::VecDeque;

        let mut disk = MockDisk {
            size: 0x4000,
            actions: VecDeque::from([
                ActionRead {
                    offset: 0x0000,
                    bytes: Vec::from([0u8; 8]),
                }
                .into(),
                ActionErase { offset: 0x108, len: 8 }.into(),
                ActionWrite {
                    offset: 0x108,
                    bytes: Vec::from([1, 2, 3, 4, 5, 6, 7, 8]),
                }
                .into(),
                ActionErase {
                    offset: 0x2010,
                    len: 16,
                }
                .into(),
                ActionWrite {
                    offset: 0x2010,
                    bytes: Vec::from([2; 16]),
                }
                .into(),
                ActionErase { offset: 0x1FF8, len: 8 }.into(),
                ActionWrite {
                    offset: 0x1FF8,
                    bytes: Vec::from([0xFF; 8]),
                }
                .into(),
            ]),
        };

        {
            let mut pm: PartitionManager<&mut MockDisk> = PartitionManager::new(&mut disk);
            let TestMap {
                mut factory,
                mut settings,
                mut slot_a,
                mut slot_b,
            } = pm.map(TestConfig);

            use block_device_driver::BlockDevice;

            let mut buf = [0u8; 8];
            let blocks = slice_to_blocks_mut(&mut buf);
            factory.read(0, blocks).await.unwrap();

            settings
                .write(0x1, slice_to_blocks(&[1, 2, 3, 4, 5, 6, 7, 8]))
                .await
                .unwrap();

            slot_b.write(0x2, slice_to_blocks(&[0x2; 16])).await.unwrap();

            // Just in bounds
            slot_a.write(0x1FF, slice_to_blocks(&[0xFF; 8])).await.unwrap();

            // Just out of bounds
            assert_eq!(
                slot_a.write(0x1FF, slice_to_blocks(&[0xFE; 16])).await,
                Err(Error::OutOfBounds)
            );

            // Completely out of bounds
            assert_eq!(
                slot_a.write(0x200, slice_to_blocks(&[0xFD; 8])).await,
                Err(Error::OutOfBounds)
            );
        }

        disk.check();
    })
}
