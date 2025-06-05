extern crate std;

use std::{
    collections::{BTreeMap, BTreeSet},
    format,
    string::ToString,
};

use crate::{Disk, Manifest, Partition};

#[test]
fn overflow() {
    let manifest = Manifest {
        variants: BTreeSet::new(),
        disk: Disk {
            size: Some(0x2900),
            alignment: None,
        },
        partitions: [
            (
                "factory".to_string(),
                Partition {
                    access: BTreeMap::new(),
                    offset: 0x0000,
                    size: 0x0100,
                },
            ),
            (
                "settings".to_string(),
                Partition {
                    access: BTreeMap::new(),
                    offset: 0x0100,
                    size: 0x0200,
                },
            ),
            (
                "slot_a".to_string(),
                Partition {
                    access: BTreeMap::new(),
                    offset: 0x1000,
                    size: 0x1000,
                },
            ),
            (
                "slot_b".to_string(),
                Partition {
                    access: BTreeMap::new(),
                    offset: 0x2000,
                    size: 0x1000,
                },
            ),
        ]
        .into(),
    };

    let result = manifest.check_consistency();

    assert_eq!(
        format!("{:?}", result),
        "Err(Partition slot_b goes over underlying disk edge)"
    );
}
