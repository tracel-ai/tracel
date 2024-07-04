use crate::backend::*;
use crate::ProcedureType;
use quote::quote;
use syn::punctuated::Punctuated;
use syn::token::Comma;
use syn::Error;
use syn::ItemFn;
use syn::Meta;

pub(crate) fn generate_inference(
    _args: &Punctuated<Meta, Comma>,
    item: &ItemFn,
) -> Result<proc_macro2::TokenStream, Vec<Error>> {
    let mut errors = Vec::<Error>::new();

    // Function name
    let fn_name = &item.sig.ident;

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

    Ok(quote! {
        #backend_types
        pub fn heat_inference_main(model: Model<#autodiff_backend_type>) {
            let device = #backend_default_device_quote;
            #fn_name::<#autodiff_backend_type>(model, device);
        }
    })
}
