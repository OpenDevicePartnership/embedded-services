use std::{fs::File, io::Read, ops::Deref, path::PathBuf};

use proc_macro::TokenStream;
use proc_macro2::Span;
use syn::{Ident, LitStr, parse::Lookahead1};

fn transform(input: Input) -> Result<proc_macro2::TokenStream, syn::Error> {
    let mut path = PathBuf::from(input.manifest.value());
    if path.is_relative() {
        let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
        path = manifest_dir.join(path);
    }

    let mut file_contents = String::new();
    File::open(&path)
        .map_err(|e| {
            syn::Error::new(
                Span::call_site(),
                format!("Could not open the manifest file at '{}': {e}", path.display()),
            )
        })?
        .read_to_string(&mut file_contents)
        .unwrap();

    let extension = path
        .extension()
        .map(|ext| ext.to_string_lossy())
        .ok_or(syn::Error::new(
            Span::call_site(),
            "Manifest file has no file extension",
        ))?;

    #[allow(unused_variables)]
    let variant_name = input.variant_name.as_ref().map(LitStr::value);

    match extension.deref() {
        #[cfg(feature = "toml")]
        "toml" => Ok(partition_manager_generation::transform_toml(
            input.name,
            input.map_name,
            variant_name,
            &file_contents,
        )),
        #[cfg(not(feature = "toml"))]
        "toml" => Err(syn::Error::new(Span::call_site(), "The toml feature is not enabled")),
        unknown => Err(syn::Error::new(
            Span::call_site(),
            format!("Unknown manifest file extension: '{unknown}'"),
        )),
    }
}

#[proc_macro]
pub fn create_partition_map(item: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(item as Input);

    match transform(input) {
        Ok(tokens) => tokens.into(),
        Err(e) => e.into_compile_error().into(),
    }
}

#[derive(Default)]
struct Preinput {
    name: Option<Ident>,
    map_name: Option<Ident>,
    variant_name: Option<LitStr>,
    manifest: Option<LitStr>,
}

#[allow(dead_code)]
struct Input {
    name: Ident,
    map_name: Ident,
    variant_name: Option<LitStr>,
    manifest: LitStr,
}

impl TryFrom<Preinput> for Input {
    type Error = syn::Error;

    fn try_from(value: Preinput) -> Result<Self, Self::Error> {
        match value {
            Preinput {
                name: Some(name),
                map_name: Some(map_name),
                variant_name,
                manifest: Some(manifest),
            } => Ok(Input {
                name,
                map_name,
                variant_name,
                manifest,
            }),
            _ => Err(syn::Error::new(
                Span::call_site(),
                "Missing fields in macro invocation: name, map_name or manifest",
            )),
        }
    }
}

/// Write a value to an option, but only if it has not yet been set before.
///
/// Emit an error with the span indicated by the look-token if it is not unique.
fn set_unique<T>(target: &mut Option<T>, value: T, look: Lookahead1<'_>) -> Result<(), syn::Error> {
    if target.replace(value).is_none() {
        Ok(())
    } else {
        Err(syn::Error::new(look.error().span(), "Duplicate field"))
    }
}

impl syn::parse::Parse for Input {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let mut result = Preinput::default();

        loop {
            let look = input.lookahead1();

            if look.peek(kw::name) {
                input.parse::<kw::name>()?;
                input.parse::<syn::Token![:]>()?;
                set_unique(&mut result.name, input.parse()?, look)?;
            } else if look.peek(kw::map_name) {
                input.parse::<kw::map_name>()?;
                input.parse::<syn::Token![:]>()?;
                set_unique(&mut result.map_name, input.parse()?, look)?;
            } else if look.peek(kw::variant) {
                input.parse::<kw::variant>()?;
                input.parse::<syn::Token![:]>()?;
                set_unique(&mut result.variant_name, input.parse()?, look)?;
            } else if look.peek(kw::manifest) {
                input.parse::<kw::manifest>()?;
                input.parse::<syn::Token![:]>()?;
                set_unique(&mut result.manifest, input.parse()?, look)?;
            } else {
                return Err(look.error());
            }

            if input.is_empty() {
                break;
            }

            input.parse::<syn::Token![,]>()?;
        }

        result.try_into()
    }
}

mod kw {
    syn::custom_keyword!(name);
    syn::custom_keyword!(map_name);
    syn::custom_keyword!(variant);
    syn::custom_keyword!(manifest);
}
