#![no_std]

#[cfg(not(target_os = "none"))]
pub use internal::*;

#[cfg(all(test, not(target_os = "none")))]
mod tests;

#[cfg(not(target_os = "none"))]
pub(crate) mod internal {
    extern crate std;

    use std::{
        collections::{BTreeMap, BTreeSet},
        ops::Range,
        string::{String, ToString},
        vec::Vec,
    };

    use anyhow::anyhow;
    use quote::quote;
    use serde::{Deserialize, Deserializer};
    use syn::Ident;

    #[derive(Debug, Clone, PartialEq, Deserialize)]
    pub struct Disk {
        pub size: Option<u32>,
        pub alignment: Option<u32>,
    }

    #[derive(Debug, Clone, PartialEq, PartialOrd, Eq, Ord)]
    pub enum Variant {
        Any,
        Other(String),
    }

    impl<'de> serde::Deserialize<'de> for Variant {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            let s = String::deserialize(deserializer)?;
            Ok(if s == "any" { Variant::Any } else { Variant::Other(s) })
        }
    }

    impl From<String> for Variant {
        fn from(value: String) -> Self {
            Variant::Other(value)
        }
    }

    impl From<&str> for Variant {
        fn from(value: &str) -> Self {
            Variant::Other(value.to_string())
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
    pub enum Access {
        #[serde(alias = "ro")]
        #[serde(alias = "read-only")]
        RO,
        #[serde(alias = "rw")]
        #[serde(alias = "read-write")]
        RW,
    }

    #[derive(Debug, Clone, PartialEq, Deserialize)]
    pub struct Partition {
        #[serde(default)]
        pub access: BTreeMap<Variant, Access>,
        pub offset: u32,
        pub size: u32,
    }

    impl From<&Partition> for Range<u32> {
        fn from(value: &Partition) -> Self {
            value.offset..value.offset + value.size
        }
    }

    fn ranges_overlap(l: &Range<u32>, r: &Range<u32>) -> bool {
        l.start < r.end && r.start < l.end
    }

    impl Partition {
        pub fn overlaps(&self, other: &Partition) -> bool {
            ranges_overlap(&self.into(), &other.into())
        }
    }

    #[derive(Debug, PartialEq)]
    pub(crate) struct GeneratedPartition {
        pub name: String,
        pub access: Access,
        pub offset: u32,
        pub size: u32,
    }

    impl GeneratedPartition {
        #[allow(dead_code)]
        pub fn name_access(self) -> (String, Access) {
            (self.name, self.access)
        }
    }

    #[derive(Debug, Clone, PartialEq, Deserialize)]
    pub struct Manifest {
        #[serde(default)]
        pub variants: BTreeSet<Variant>,
        pub disk: Disk,
        #[serde(deserialize_with = "deserialize_partitions")]
        pub partitions: BTreeMap<String, Partition>,
    }

    // Implement deserialization of partitions such that duplicate names are checked.
    fn deserialize_partitions<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<BTreeMap<String, Partition>, D::Error> {
        struct MapVisitor;

        impl<'de> serde::de::Visitor<'de> for MapVisitor {
            type Value = BTreeMap<String, Partition>;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a map")
            }

            #[inline]
            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::MapAccess<'de>,
            {
                let mut values = BTreeMap::new();

                while let Some((key, value)) = map.next_entry::<String, Partition>()? {
                    if values.insert(key.clone(), value).is_some() {
                        return Err(serde::de::Error::custom(std::format!(
                            "Duplicate key {key} in partitions",
                        )));
                    }
                }

                Ok(values)
            }
        }

        deserializer.deserialize_map(MapVisitor)
    }

    impl Manifest {
        fn validate_alignment(&self) -> anyhow::Result<()> {
            if let Some(alignment) = self.disk.alignment {
                for (name, partition) in self.partitions.iter() {
                    if partition.offset % alignment != 0 || partition.size % alignment != 0 {
                        return Err(anyhow!("Partition {} is not aligned to {} bytes", name, alignment));
                    }
                }
            }
            Ok(())
        }

        fn validate_overlap(&self) -> anyhow::Result<()> {
            let mut partitions = Vec::from_iter(self.partitions.iter());
            partitions.sort_by_key(|(_, partition)| partition.offset);

            for (i, (partition_name_x, partition_x)) in partitions.iter().enumerate() {
                for (partition_name_y, partition_y) in partitions.iter().skip(i + 1) {
                    if partition_x.overlaps(partition_y) {
                        return Err(anyhow!(
                            "Partitions {} and {} overlap",
                            partition_name_x,
                            partition_name_y
                        ));
                    }
                }
            }

            Ok(())
        }

        fn validate_size(&self) -> anyhow::Result<()> {
            if let Some(size) = self.disk.size {
                for (name, partition) in self.partitions.iter() {
                    if partition.offset + partition.size > size {
                        return Err(anyhow!("Partition {} goes over underlying disk edge", name));
                    }
                }
            }
            Ok(())
        }

        pub fn check_consistency(&self) -> anyhow::Result<()> {
            self.validate_size()?;
            self.validate_overlap()?;
            self.validate_alignment()?;
            Ok(())
        }

        pub(crate) fn generate(
            self,
            variant_name: Option<String>,
        ) -> anyhow::Result<impl Iterator<Item = GeneratedPartition>> {
            self.check_consistency()?;

            let variant_name = match variant_name {
                Some(variant_name) => {
                    if !self.variants.contains(&Variant::Other(variant_name.clone())) {
                        return Err(anyhow!("Variant '{}' not defined in manifest", variant_name));
                    }
                    Some(variant_name)
                }
                None => None,
            };

            let variant = variant_name.map(Variant::Other).unwrap_or(Variant::Any);

            Ok(self
                .partitions
                .into_iter()
                .filter_map(move |(name, Partition { access, offset, size })| {
                    let access = match access.get(&variant).or_else(|| access.get(&Variant::Any)) {
                        Some(Access::RO) => Access::RO,
                        Some(Access::RW) => Access::RW,
                        None if access.is_empty() => Access::RW, // Nothing specified, assume RW for all.
                        None => return None,                     // No-match, do not emit partition.
                    };

                    Some(GeneratedPartition {
                        name,
                        access,
                        offset,
                        size,
                    })
                }))
        }
    }

    #[cfg(feature = "toml")]
    pub fn transform_toml(
        name: Ident,
        map_name: Ident,
        variant_name: Option<String>,
        manifest: &str,
    ) -> proc_macro2::TokenStream {
        let manifest = match transform_toml_manifest(manifest) {
            Ok(manifest) => manifest,
            Err(e) => return anyhow_error_to_compile_error(e),
        };

        transform_manifest(name, map_name, variant_name, manifest)
    }

    #[cfg(feature = "toml")]
    pub(crate) fn transform_toml_manifest(manifest: &str) -> anyhow::Result<Manifest> {
        Ok(toml::from_str(manifest)?)
    }

    pub fn transform_manifest(
        name: Ident,
        map_name: Ident,
        variant_name: Option<String>,
        manifest: Manifest,
    ) -> proc_macro2::TokenStream {
        let partitions = Vec::from_iter(match manifest.generate(variant_name) {
            Ok(partitions) => partitions,
            Err(e) => return anyhow_error_to_compile_error(e),
        });

        let partitions_def = partitions.iter().map(|partition| {
            let partition_name = quote::format_ident!("{}", partition.name);

            let access = match partition.access {
                Access::RO => quote! { partition_manager::RO },
                Access::RW => quote! { partition_manager::RW },
            };

            quote! { pub #partition_name: partition_manager::Partition<'a, F, #access, M>, }
        });

        let partitions_constr = partitions.iter().map(|partition| {
            let offset = partition.offset;
            let size = partition.size;
            let name = quote::format_ident!("{}", partition.name);

            quote! { #name: partition_manager::Partition::new(storage, #offset, #size), }
        });

        quote! {
            pub struct #name {
                /// Private constructor
                _inner: (),
            }

            impl #name {
                pub const fn new() -> Self {
                    Self { _inner: () }
                }
            }

            pub struct #map_name<'a, F, M: embassy_sync::blocking_mutex::raw::RawMutex = embassy_sync::blocking_mutex::raw::NoopRawMutex> {
                #(#partitions_def)*
            }

            impl<'a, F, M: embassy_sync::blocking_mutex::raw::RawMutex> partition_manager::PartitionMap for #map_name<'a, F, M> {}

            impl partition_manager::PartitionConfig for #name {
                type Map<'a, F, M: embassy_sync::blocking_mutex::raw::RawMutex>
                    = #map_name<'a, F, M>
                where
                    F: 'a,
                    M: 'a;

                fn map<F, M: embassy_sync::blocking_mutex::raw::RawMutex>(
                    self,
                    storage: &embassy_sync::mutex::Mutex<M, F>,
                ) -> Self::Map<'_, F, M> {
                    #map_name {
                        #(#partitions_constr)*
                    }
                }
            }
        }
    }

    fn anyhow_error_to_compile_error(error: anyhow::Error) -> proc_macro2::TokenStream {
        syn::Error::new(proc_macro2::Span::call_site(), std::format!("{error:#}")).into_compile_error()
    }
}
