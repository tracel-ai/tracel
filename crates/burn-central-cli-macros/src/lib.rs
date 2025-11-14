#[allow(dead_code)]
mod name_value;

use proc_macro::TokenStream;
use proc_macro2::Ident;
use quote::quote;

use strum::Display;
use syn::{Error, ItemFn, Meta, Path, parse_macro_input, punctuated::Punctuated, spanned::Spanned};

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
    builder_fn_ident: &Ident,
    procedure_type: &ProcedureType,
    routine_name: &syn::LitStr,
) -> proc_macro2::TokenStream {
    let fn_name = &item.sig.ident;
    let builder_fn_name = &builder_fn_ident;
    let serialized_fn_item = syn_serde::json::to_string(item);
    let serialized_lit_arr = syn::LitByteStr::new(serialized_fn_item.as_bytes(), item.span());
    let proc_type_str = Ident::new(
        &procedure_type.to_string().to_lowercase(),
        proc_macro2::Span::call_site(),
    );
    quote! {
        burn_central::cli::register_functions!(
            burn_central::cli::tools::functions_registry::FunctionMetadata,
            burn_central::cli::tools::functions_registry::FunctionMetadata::new(
                module_path!(),
                stringify!(#fn_name),
                stringify!(#builder_fn_name),
                #routine_name,
                stringify!(#proc_type_str),
                #serialized_lit_arr));
    }
}

fn get_string_arg(
    args: &Punctuated<Meta, syn::Token![,]>,
    arg_name: &str,
    errors: &mut Vec<Error>,
) -> Option<syn::LitStr> {
    args.iter()
        .find(|meta| meta.path().is_ident(arg_name))
        .and_then(|meta| match meta.require_name_value() {
            Ok(value) => match &value.value {
                syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Str(name),
                    ..
                }) => Some(name.clone()),
                _ => {
                    errors.push(Error::new(
                        value.value.span(),
                        format!("Expected a string literal for the `{arg_name}` argument."),
                    ));
                    None
                }
            },
            Err(err) => {
                errors.push(err);
                None
            }
        })
}

fn validate_registered_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("Registered name cannot be empty.".to_string());
    }
    if name.contains(' ') {
        return Err("Registered name cannot contain spaces.".to_string());
    }
    if !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return Err(
            "Registered name can only contain alphanumeric characters and underscores.".to_string(),
        );
    }
    Ok(())
}

#[proc_macro_attribute]
pub fn register(args: TokenStream, item: TokenStream) -> TokenStream {
    let mut errors = Vec::<Error>::new();
    let args = parse_macro_input!(args with Punctuated::<Meta, syn::Token![,]>::parse_terminated);
    let item = parse_macro_input!(item as ItemFn);
    let fn_name = &item.sig.ident;

    if args.is_empty() {
        errors.push(Error::new(
            args.span(),
            "Expected one argument for the #[register] attribute. Please provide the procedure type (training or inference) as the first argument.",
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

    if procedure_type == ProcedureType::Inference {
        errors.push(Error::new_spanned(
            args.first().unwrap().path(),
            "Inference procedures are not supported yet. Please use training procedures.",
        ));
    }

    let maybe_registered_name = get_string_arg(&args, "name", &mut errors);

    if let Some(name) = &maybe_registered_name {
        if let Err(err) = validate_registered_name(&name.value()) {
            errors.push(Error::new_spanned(
                name,
                format!("Invalid registered name: {err}"),
            ));
        }
    }

    let builder_fn_name = syn::Ident::new(
        &format!("__{fn_name}_builder"),
        proc_macro2::Span::call_site(),
    );

    let registered_name_str = {
        let name = maybe_registered_name
            .map(|name| name.value())
            .unwrap_or_else(|| fn_name.to_string());

        syn::LitStr::new(&name, fn_name.span())
    };

    let builder_item = match procedure_type {
        ProcedureType::Training => {
            quote! {
                #[doc(hidden)]
                pub fn #builder_fn_name<B: burn::tensor::backend::AutodiffBackend>(
                    exec: &mut burn_central::runtime::ExecutorBuilder<B>,
                ) {
                    exec.train(#registered_name_str, #fn_name);
                }
            }
        }
        ProcedureType::Inference => {
            quote! {}
        }
    };

    let flag_register = generate_flag_register_stream(
        &item,
        &builder_fn_name,
        &procedure_type,
        &registered_name_str,
    );

    let code = quote! {
        #[allow(dead_code)]
        #item

        #flag_register
        #builder_item
    };

    // If there are any errors, combine them and return
    if !errors.is_empty() {
        compile_errors(errors).into()
    } else {
        code.into()
    }
}

#[proc_macro_attribute]
pub fn burn_central_main(args: TokenStream, item: TokenStream) -> TokenStream {
    let item = parse_macro_input!(item as ItemFn);
    let args: Punctuated<Meta, syn::token::Comma> =
        parse_macro_input!(args with Punctuated::<Meta, syn::Token![,]>::parse_terminated);

    let module_path = args
        .first()
        .expect("Should be able to get first arg.")
        .path()
        .clone();

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

    let main_call = if option_env!("COMPUTE_PROVIDER_RUNTIME").is_some() {
        quote! {
            burn_central::cli::compute_provider::compute_provider_main();
        }
    } else {
        quote! {
            burn_central::cli::cli::cli_main();
        }
    };

    let item = quote! {
        mod #mod_tokens {
            #[allow(unused_imports)]
            pub use #module_path;
        }

        #item_sig {
            #main_call
        }
    };

    let code = quote! {
        #item
    };

    code.into()
}
