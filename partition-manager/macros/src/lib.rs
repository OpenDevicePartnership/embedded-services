use std::{fs::File, io::Read, ops::Deref, path::PathBuf};

use proc_macro::TokenStream;
use proc_macro2::Span;
use syn::{Ident, LitStr};

fn transform(input: Input) -> Result<proc_macro2::TokenStream, syn::Error> {
    {
        let mut path = PathBuf::from(input.path.value());
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

        let variant_name = input.variant_name.as_ref().map(LitStr::value);

        match extension.deref() {
            #[cfg(feature = "json")]
            "json" => Ok(partition_manager_generation::transform_json(
                input.name,
                input.map_name,
                variant_name,
                &file_contents,
            )),
            #[cfg(not(feature = "json"))]
            "json" => Err(syn::Error::new(
                Span::call_site(),
                format!("The json feature is not enabled"),
            )),
            #[cfg(feature = "toml")]
            "toml" => Ok(partition_manager_generation::transform_toml(
                input.name,
                input.map_name,
                variant_name,
                &file_contents,
            )),
            #[cfg(not(feature = "toml"))]
            "toml" => Err(syn::Error::new(
                Span::call_site(),
                format!("The toml feature is not enabled"),
            )),
            unknown => Err(syn::Error::new(
                Span::call_site(),
                format!("Unknown manifest file extension: '{unknown}'"),
            )),
        }
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

struct Input {
    name: Ident,
    map_name: Ident,
    variant_name: Option<LitStr>,
    path: LitStr,
}

impl syn::parse::Parse for Input {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        input.parse::<kw::name>()?;
        input.parse::<syn::Token![:]>()?;
        let name = input.parse()?;
        input.parse::<syn::Token![,]>()?;
        input.parse::<kw::map_name>()?;
        input.parse::<syn::Token![:]>()?;
        let map_name = input.parse()?;
        input.parse::<syn::Token![,]>()?;

        let look = input.lookahead1();
        let variant_name = if look.peek(kw::variant) {
            input.parse::<kw::variant>()?;
            input.parse::<syn::Token![:]>()?;
            let variant_name = input.parse()?;
            input.parse::<syn::Token![,]>()?;

            Some(variant_name)
        } else {
            None
        };

        input.parse::<kw::manifest>()?;
        input.parse::<syn::Token![:]>()?;

        let path = input.parse()?;

        Ok(Self {
            name,
            map_name,
            variant_name,
            path,
        })
    }
}

mod kw {
    syn::custom_keyword!(name);
    syn::custom_keyword!(map_name);
    syn::custom_keyword!(variant);
    syn::custom_keyword!(manifest);
}
