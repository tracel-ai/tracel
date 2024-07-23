use proc_macro::TokenStream;
use quote::quote;

use strum::Display;
use syn::{parse_macro_input, punctuated::Punctuated, spanned::Spanned, Error, ItemFn, Meta, Path};

#[derive(Eq, Hash, PartialEq, Display)]
#[strum(serialize_all = "PascalCase")]
pub(crate) enum ProcedureType {
    Training,
    Inference,
}

impl TryFrom<Path> for ProcedureType {
    type Error = Error;

    fn try_from(path: Path) -> Result<Self, Self::Error> {
        match path.get_ident() {
            Some(ident) => match ident.to_string().as_str() {
                "training" => Ok(Self::Training),
                "inference" => Ok(Self::Inference),
                _ => Err(Error::new_spanned(
                    path,
                    "Expected `training` or `inference`",
                )),
            },
            None => Err(Error::new_spanned(
                path,
                "Expected `training` or `inference`",
            )),
        }
    }
}

fn compile_errors(errors: Vec<Error>) -> proc_macro2::TokenStream {
    errors
        .into_iter()
        .map(|err| err.to_compile_error())
        .collect()
}

pub(crate) fn generate_flag_register_stream(
    item: &ItemFn,
    procedure_type: &ProcedureType,
) -> proc_macro2::TokenStream {
    let fn_name = &item.sig.ident;
    let serialized_fn_item = syn_serde::json::to_string(item);
    let serialized_lit_arr = syn::LitByteStr::new(serialized_fn_item.as_bytes(), item.span());
    let proc_type_str = syn::Ident::new(
        &procedure_type.to_string().to_lowercase(),
        proc_macro2::Span::call_site(),
    );
    quote! {
        tracel::heat::sdk_cli::register_flag!(
            tracel::heat::sdk_cli::registry::Flag,
            tracel::heat::sdk_cli::registry::Flag::new(
                module_path!(),
                stringify!(#fn_name),
                stringify!(#proc_type_str),
                #serialized_lit_arr));
    }
}

#[proc_macro_attribute]
pub fn heat(args: TokenStream, item: TokenStream) -> TokenStream {
    let project_dir = std::env::var("HEAT_PROJECT_DIR");
    if project_dir.is_ok() {
        let item: proc_macro2::TokenStream = item.into();
        return quote! {
            #[allow(dead_code)]
            #item
        }
        .into();
    }

    let mut errors = Vec::<Error>::new();
    let args = parse_macro_input!(args with Punctuated::<Meta, syn::Token![,]>::parse_terminated);
    let item = parse_macro_input!(item as ItemFn);

    if args.len() != 1 {
        errors.push(Error::new(
            args.span(),
            "Expected one argument for the #[heat] attribute. Please provide `training` or `inference`.",
        ));
    }

    // Determine the proc type (training or inference)
    let procedure_type = match ProcedureType::try_from(
        args.first()
            .expect("Should be able to get first arg.")
            .path()
            .clone(),
    ) {
        Ok(procedure_type) => procedure_type,
        Err(err) => {
            return err.into_compile_error().into();
        }
    };

    // If there are any errors, combine them and return
    if !errors.is_empty() {
        return compile_errors(errors).into();
    }

    let flag_register = if cfg!(feature = "build-cli") {
        generate_flag_register_stream(&item, &procedure_type)
    } else {
        quote! {}
    };

    quote! {
        #[allow(dead_code)]
        #item

        #flag_register
    }
    .into()
}

#[cfg(feature = "build-cli")]
#[proc_macro_attribute]
pub fn heat_cli_main(args: TokenStream, item: TokenStream) -> TokenStream {
    let item = parse_macro_input!(item as ItemFn);

    let module_path = parse_macro_input!(args as Path); // Parse the module path

    let item_sig = &item.sig;
    let item_block = &item.block;

    // cause an error if the function has a body
    if !item_block.stmts.is_empty() {
        return Error::new(
            item_block.span(),
            "The cli main function should not have a body",
        )
        .to_compile_error()
        .into();
    }

    let mod_tokens = syn::Ident::new(
        &format!("__{}", uuid::Uuid::new_v4().simple()),
        proc_macro2::Span::call_site(),
    );
    let item = quote! {
        mod #mod_tokens {
            #[allow(unused_imports)]
            pub use #module_path;
        }

        #item_sig {
            tracel::heat::sdk_cli::cli::cli_main();
        }
    };

    quote! {
        #item
    }
    .into()
}

///
/// Usage example:
/// ```rust
/// #[heat_import_extern_crate(crate_name)]
///
#[proc_macro_attribute]
pub fn heat_import_extern_crate(_args: TokenStream, item: TokenStream) -> TokenStream {
    let item = parse_macro_input!(item as ItemFn);

    let item_sig = &item.sig;
    let item_block = &item.block;

    // cause an error if the function has a body
    if !item_block.stmts.is_empty() {
        return Error::new(
            item_block.span(),
            "The import_extern_crate function should not have a body",
        )
        .to_compile_error()
        .into();
    }

    let item = quote! {
        #item_sig {
            tracel::heat::sdk_cli::cli::import_extern_crate();
        }
    };

    quote! {
        #item
    }
    .into()
}
