use crate::{
    Error, PartitionManager,
    test::{
        TestConfig, TestMap,
        mock::{self, ActionErase, ActionRead, ActionWrite, MockDisk},
    },
};

#[test]
fn esa() {
    embassy_futures::block_on(async {
        use std::collections::VecDeque;

        let mut disk = MockDisk {
            size: 0x4000,
            actions: VecDeque::from([
                ActionRead {
                    offset: 0x0004,
                    bytes: Vec::from([0u8; 8]),
                }
                .into(),
                ActionWrite {
                    offset: 0x180,
                    bytes: Vec::from(core::array::from_fn::<u8, 128, _>(|i| i as u8)),
                }
                .into(),
                ActionErase {
                    offset: 0x2000,
                    len: 0x1000,
                }
                .into(),
                ActionWrite {
                    offset: 0x1F80,
                    bytes: Vec::from([0xFF; 128]),
                }
                .into(),
            ]),
        };

        {
            let mut pm: PartitionManager<_> = PartitionManager::new(&mut disk);
            let TestMap {
                mut factory,
                mut settings,
                mut slot_a,
                mut slot_b,
            } = pm.map(TestConfig);

            use embedded_storage_async::nor_flash::{NorFlash, ReadNorFlash};

            let mut buf = [0u8; 8];
            factory.read(4, &mut buf).await.unwrap();
            settings
                .write(0x80, &core::array::from_fn::<u8, 128, _>(|i| i as u8))
                .await
                .unwrap();
            slot_b.erase(0x0000, slot_b.capacity() as u32).await.unwrap();

            // Just in bounds
            slot_a.write(0x0F80, &[0xFF; 128]).await.unwrap();

            // Underlying not aligned
            assert_eq!(
                slot_a.write(0x0FFF, &[0xFE]).await,
                Err(Error::Inner(mock::esa::Error::NotAligned))
            );

            // Just out of bounds
            assert_eq!(slot_a.write(0x0FFF, &[0xFE, 0xFD]).await, Err(Error::OutOfBounds));

            // Completely out of bounds
            assert_eq!(slot_a.write(0x1000, &[0xFE, 0xFD]).await, Err(Error::OutOfBounds));
        }

        disk.check();
    })
}
