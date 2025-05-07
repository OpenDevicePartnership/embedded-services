extern crate std;

use std::{
    collections::{BTreeMap, BTreeSet},
    format,
    string::ToString,
    vec,
    vec::Vec,
};

use crate::{Access, Disk, GeneratedPartition, Manifest, Partition, Variant};

fn create_manifest() -> Manifest {
    Manifest {
        variants: BTreeSet::from_iter(["bootloader".into(), "application".into()]),
        disk: Disk {
            size: None,
            alignment: None,
        },
        partitions: [(
            "l1".to_string(),
            Partition {
                access: BTreeMap::new(),
                offset: 0x0000,
                size: 0x0100,
            },
        )]
        .into(),
    }
}

fn create_generated_manifest() -> Vec<GeneratedPartition> {
    vec![GeneratedPartition {
        name: "l1".to_string(),
        access: crate::Access::RW,
        offset: 0x0000,
        size: 0x0100,
    }]
}

#[test]
fn variant_none() {
    let result = create_manifest().generate(None).map(Vec::from_iter).unwrap();
    assert_eq!(result, create_generated_manifest());
}

#[test]
fn variant_unspecified() {
    let result = create_manifest()
        .generate(Some("Unspecified".to_string()))
        .map(Vec::from_iter);

    assert_eq!(
        format!("{:?}", result),
        "Err(Variant 'Unspecified' not defined in manifest)"
    );
}

#[test]
fn variant_no_match() {
    let mut manifest = create_manifest();
    manifest.partitions.get_mut("l1").unwrap().access = BTreeMap::from_iter([("bootloader".into(), Access::RW)]);

    let result = manifest
        .generate(Some("application".to_string()))
        .map(Vec::from_iter)
        .unwrap();

    assert_eq!(result, Vec::new());
}

#[test]
fn variant_match() {
    let mut manifest = create_manifest();
    manifest.partitions.get_mut("l1").unwrap().access = BTreeMap::from_iter([("bootloader".into(), Access::RW)]);

    let result = manifest
        .generate(Some("bootloader".to_string()))
        .map(Vec::from_iter)
        .unwrap();

    assert_eq!(result, create_generated_manifest());
}

#[test]
fn variant_match_from_multiple() {
    let manifest = Manifest {
        variants: BTreeSet::from_iter(["bootloader".into(), "application".into()]),
        disk: Disk {
            size: None,
            alignment: None,
        },
        partitions: [
            (
                "l1".to_string(),
                Partition {
                    access: BTreeMap::from_iter([(Variant::Any, Access::RO)]),
                    offset: 0x0000,
                    size: 0x0100,
                },
            ),
            (
                "app".to_string(),
                Partition {
                    access: BTreeMap::from_iter([(Variant::Any, Access::RO), ("bootloader".into(), Access::RW)]),
                    offset: 0x1000,
                    size: 0x01000,
                },
            ),
        ]
        .into(),
    };

    let result = BTreeMap::from_iter(
        manifest
            .clone()
            .generate(Some("bootloader".to_string()))
            .map(Vec::from_iter)
            .unwrap()
            .into_iter()
            .map(GeneratedPartition::name_access),
    );

    assert_eq!(
        result,
        BTreeMap::from_iter([("l1".to_string(), Access::RO), ("app".to_string(), Access::RW)])
    );

    let result = BTreeMap::from_iter(
        manifest
            .clone()
            .generate(Some("application".to_string()))
            .map(Vec::from_iter)
            .unwrap()
            .into_iter()
            .map(GeneratedPartition::name_access),
    );

    assert_eq!(
        result,
        BTreeMap::from_iter([("l1".to_string(), Access::RO), ("app".to_string(), Access::RO)])
    );
}

#[test]
fn full() {
    let manifest = Manifest {
        variants: BTreeSet::from_iter(["bootloader".into(), "application".into()]),
        disk: Disk {
            size: Some(0x4000),
            alignment: Some(0x0100),
        },
        partitions: [
            (
                "factory".to_string(),
                Partition {
                    access: BTreeMap::from_iter([(Variant::Any, Access::RO)]),
                    offset: 0x0000,
                    size: 0x0100,
                },
            ),
            // Settings is hidden for bootloader, as it is irrelevant.
            (
                "settings".to_string(),
                Partition {
                    access: BTreeMap::from_iter([("application".into(), Access::RW)]),
                    offset: 0x0100,
                    size: 0x0100,
                },
            ),
            (
                "l1_state".to_string(),
                Partition {
                    access: BTreeMap::from_iter([
                        ("bootloader".into(), Access::RW),
                        ("application".into(), Access::RO),
                    ]),
                    offset: 0x0200,
                    size: 0x0200,
                },
            ),
            // L1 code should not be leaked to application.
            (
                "l1".to_string(),
                Partition {
                    access: BTreeMap::from_iter([("bootloader".into(), Access::RO)]),
                    offset: 0x0400,
                    size: 0x0800,
                },
            ),
            (
                "slot_a".to_string(),
                Partition {
                    access: BTreeMap::from_iter([(Variant::Any, Access::RO), ("bootloader".into(), Access::RW)]),
                    offset: 0x1000,
                    size: 0x01000,
                },
            ),
            (
                "slot_b".to_string(),
                Partition {
                    access: BTreeMap::from_iter([(Variant::Any, Access::RW)]),
                    offset: 0x2000,
                    size: 0x01000,
                },
            ),
        ]
        .into(),
    };

    let result = BTreeMap::from_iter(
        manifest
            .clone()
            .generate(Some("bootloader".to_string()))
            .map(Vec::from_iter)
            .unwrap()
            .into_iter()
            .map(GeneratedPartition::name_access),
    );

    assert_eq!(
        result,
        BTreeMap::from_iter([
            ("factory".to_string(), Access::RO),
            ("l1_state".to_string(), Access::RW),
            ("l1".to_string(), Access::RO),
            ("slot_a".to_string(), Access::RW),
            ("slot_b".to_string(), Access::RW)
        ])
    );

    let result = BTreeMap::from_iter(
        manifest
            .clone()
            .generate(Some("application".to_string()))
            .map(Vec::from_iter)
            .unwrap()
            .into_iter()
            .map(GeneratedPartition::name_access),
    );

    assert_eq!(
        result,
        BTreeMap::from_iter([
            ("factory".to_string(), Access::RO),
            ("settings".to_string(), Access::RW),
            ("l1_state".to_string(), Access::RO),
            ("slot_a".to_string(), Access::RO),
            ("slot_b".to_string(), Access::RW)
        ])
    );
}
