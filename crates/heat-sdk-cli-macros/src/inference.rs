use crate::backend::*;
use crate::ProcedureType;
use quote::quote;
use syn::parse_quote;
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::token::Comma;
use syn::Error;
use syn::GenericArgument;
use syn::Ident;
use syn::ItemFn;
use syn::Meta;
use syn::PathArguments;
use syn::Type;
use syn::TypePath;

/// Returns the module type used for the inference function injected with the backend of the function.
fn get_inference_module_type(item: &ItemFn, new_backend: &Ident) -> Option<GenericArgument> {
    if let Some(syn::FnArg::Typed(pat_type)) = item.sig.inputs.first() {
        if let Type::Path(TypePath { path, .. }) = &*pat_type.ty {
            if let PathArguments::AngleBracketed(_) = &path.segments.last().unwrap().arguments {
                let model_type = &path.segments.last().unwrap().ident;
                let new_model_type: syn::Type = parse_quote! { #model_type<#new_backend> };
                let new_result_type = GenericArgument::Type(new_model_type);
                return Some(new_result_type);
            }
        }
    }

    None
}

pub(crate) fn generate_inference(
    _args: &Punctuated<Meta, Comma>,
    item: &ItemFn,
) -> Result<proc_macro2::TokenStream, Vec<Error>> {
    let mut errors = Vec::<Error>::new();

    // Extract signature information
    let fn_name = &item.sig.ident;
    let fn_generics = &item.sig.generics;

    // Enforce backend generic (should be exactly one generic parameter named `B` for the backend type)
    if let Err(err) = enforce_fn_backend_generic(fn_generics) {
        errors.push(err);
    }

    // Select backend
    let backend = match get_backend_type(item) {
        Ok(backend) => backend,
        Err(err) => {
            errors.push(err);
            return Err(errors);
        }
    };
    let backend_types = generate_backend_typedef_stream(&backend, &ProcedureType::Inference);
    let (_backend_type, autodiff_backend_type) = get_backend_type_names(&ProcedureType::Inference);
    let backend_default_device_quote = backend.default_device_stream();

    // Get the parameter module type (first arg) and insert the generated backend type generic argument.
    // This makes it possible for the generated function to accept the same concrete module type as the original function as arg.
    let modified_module_type = get_inference_module_type(item, &autodiff_backend_type);

    if modified_module_type.is_none() {
        errors.push(Error::new(
            item.sig.output.span(),
            "Expected first parameter type to be a module type with Backend parameter.",
        ));
    }

    if !errors.is_empty() {
        return Err(errors);
    }

    Ok(quote! {
        #backend_types
        pub fn heat_inference_main(model: #modified_module_type) {
            let device = #backend_default_device_quote;
            let item = burn::data::dataset::vision::MnistDataset::test()
            .get(42)
            .unwrap();
            #fn_name::<#autodiff_backend_type>(model, device, item)
        }
    })
}
