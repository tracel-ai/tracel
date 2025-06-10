use quote::quote;
use strum::{Display, EnumString};
use syn::Ident;

#[allow(dead_code)]
#[derive(Debug, Clone, Display, EnumString)]
#[strum(serialize_all = "snake_case")]
pub enum BackendType {
    Wgpu,
    Tch,
    Ndarray,
}

impl BackendType {
    /// Returns the token stream for the default device for the backend.
    pub fn default_device_stream(&self) -> proc_macro2::TokenStream {
        match self {
            BackendType::Wgpu => {
                quote! {
                    burn::backend::wgpu::WgpuDevice::default()
                }
            }
            BackendType::Tch => {
                quote! {
                    burn::backend::libtorch::LibTorchDevice::default()
                }
            }
            BackendType::Ndarray => {
                quote! {
                    burn::backend::ndarray::NdArrayDevice::default()
                }
            }
        }
    }

    pub fn backend_stream(&self) -> proc_macro2::TokenStream {
        match self {
            BackendType::Wgpu => {
                quote! {burn::backend::Wgpu<f32, i32>}
            }
            BackendType::Tch => {
                quote! {burn::backend::libtorch::LibTorch<f32>}
            }
            BackendType::Ndarray => {
                quote! {burn::backend::ndarray::NdArray<f32>}
            }
        }
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
    let backend_type = backend.backend_stream();

    quote! {
        type #backend_type_name = #backend_type;
        type #autodiff_backend_type_name = burn::backend::Autodiff<#backend_type_name>;
    }
}
