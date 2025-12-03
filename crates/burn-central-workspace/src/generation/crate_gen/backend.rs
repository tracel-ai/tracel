use quote::quote;
use syn::Ident;

use crate::execution::BackendType;

fn backend_to_token_stream(backend: &BackendType) -> proc_macro2::TokenStream {
    match backend {
        BackendType::Wgpu => quote! { burn::backend::Wgpu<f32, i32> },
        BackendType::Tch => quote! { burn::backend::libtorch::LibTorch<f32> },
        BackendType::Ndarray => quote! { burn::backend::ndarray::NdArray<f32> },
    }
}

pub fn default_device_stream() -> proc_macro2::TokenStream {
    quote! {
        Default::default()
    }
}

pub fn get_burn_feature_flags(backend: &BackendType) -> Vec<&'static str> {
    match backend {
        BackendType::Wgpu => vec!["wgpu"],
        BackendType::Tch => vec!["tch"],
        BackendType::Ndarray => vec!["ndarray"],
    }
}

/// Returns the backend type names for the given procedure type.
/// Ex: For ProcedureType::Training, the backend type name will be MyTrainingBackend and autodiff backend type name will be MyTrainingAutodiffBackend.
pub(crate) fn get_backend_type_names() -> (syn::Ident, syn::Ident) {
    let backend = "MyBackend";
    let autodiff_backend = "MyAutodiffBackend";
    let backend_type_name = Ident::new(backend, proc_macro2::Span::call_site());
    let autodiff_backend_type_name = Ident::new(autodiff_backend, proc_macro2::Span::call_site());
    (backend_type_name, autodiff_backend_type_name)
}

/// Creates the stream of tokens that creates the type aliases for the backend and corresponding autodiff backend.
pub(crate) fn generate_backend_typedef_stream(backend: &BackendType) -> proc_macro2::TokenStream {
    let (backend_type_name, autodiff_backend_type_name) = get_backend_type_names();
    let backend_type = backend_to_token_stream(backend);

    quote! {
        type #backend_type_name = #backend_type;
        type #autodiff_backend_type_name = burn::backend::Autodiff<#backend_type_name>;
    }
}
