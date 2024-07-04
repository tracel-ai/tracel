use inference::generate_inference;
use proc_macro::TokenStream;
use quote::quote;

use strum::Display;
use syn::{parse_macro_input, punctuated::Punctuated, spanned::Spanned, Error, ItemFn, Meta, Path};
use training::generate_training;

mod backend;
mod inference;
mod training;

#[derive(Eq, Hash, PartialEq, Display)]
#[strum(serialize_all = "PascalCase")]
pub(crate) enum ProcedureType {
    Training,
    Inference,
    Setup,
}

impl TryFrom<Path> for ProcedureType {
    type Error = Error;

    fn try_from(path: Path) -> Result<Self, Self::Error> {
        match path.get_ident() {
            Some(ident) => match ident.to_string().as_str() {
                "training" => Ok(Self::Training),
                "inference" => Ok(Self::Inference),
                "setup" => Ok(Self::Setup),
                _ => Err(Error::new_spanned(
                    path,
                    "Expected `training`, `inference` or `setup`",
                )),
            },
            None => Err(Error::new_spanned(
                path,
                "Expected `training`, `inference` or `setup`",
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

#[proc_macro_attribute]
pub fn heat(args: TokenStream, item: TokenStream) -> TokenStream {
    let mut errors = Vec::<Error>::new();
    let args = parse_macro_input!(args with Punctuated::<Meta, syn::Token![,]>::parse_terminated);
    let item = parse_macro_input!(item as ItemFn);

    if args.len() != 1 {
        errors.push(Error::new(
            args.span(),
            "Expected one argument for the #[heat] attribute. Please provide `training`, `inference` or `setup`.",
        ));
    }

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

    let mut generated_code = match procedure_type {
        ProcedureType::Training => generate_training(&args, &item),
        ProcedureType::Inference => generate_inference(&args, &item),
        _ => {
            unimplemented!()
        }
    };

    if let Err(ref mut err) = generated_code {
        errors.append(err);
        return compile_errors(errors).into();
    }
    let generated_code = generated_code.expect("Should be able to generate code.");

    // If there are any errors, combine them and return
    if !errors.is_empty() {
        return compile_errors(errors).into();
    }

    let fn_name = &item.sig.ident;
    let proc_type_str =
        syn::Ident::new(&procedure_type.to_string(), proc_macro2::Span::call_site());
    quote! {
        #item
        #generated_code

        tracel::heat::register_flag!(
            tracel::heat::Flag,
            tracel::heat::Flag::new(
                Box::leak(format!("{}::{}", module_path!(), stringify!(#fn_name)).into_boxed_str())
                , stringify!(#proc_type_str)));
    }
    .into()
}

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
            pub extern crate #module_path;
        }

        #item_sig {
            tracel::heat::cli::cli_main();
        }
    };

    quote! {
        #item
    }
    .into()
}