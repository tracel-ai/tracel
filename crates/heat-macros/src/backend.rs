use crate::ProcedureType;
use quote::quote;
use syn::Error;
use syn::{Ident, ItemFn};

#[allow(dead_code)]
#[derive(Clone)]
pub(crate) enum BackendType {
    Wgpu,
    Tch,
    Ndarray,
}

impl BackendType {
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

static DEFAULT_BACKEND: BackendType = BackendType::Wgpu;

/// Returns the backend type names for the given procedure type.
/// Ex: For ProcedureType::Training, the backend type name will be MyTrainingBackend and autodiff backend type name will be MyTrainingAutodiffBackend.
pub(crate) fn get_backend_type_names(proc_type: &ProcedureType) -> (syn::Ident, syn::Ident) {
    let backend = format!("My{}Backend", proc_type);
    let autodiff_backend = format!("My{}AutodiffBackend", proc_type);
    let backend_type_name = Ident::new(&backend, proc_macro2::Span::call_site());
    let autodiff_backend_type_name = Ident::new(&autodiff_backend, proc_macro2::Span::call_site());
    (backend_type_name, autodiff_backend_type_name)
}

/// Creates the stream of tokens that creates the type aliases for the backend and corresponding autodiff backend.
pub(crate) fn generate_backend_typedef_stream(
    backend: &BackendType,
    proc_type: &ProcedureType,
) -> proc_macro2::TokenStream {
    let (backend_type_name, autodiff_backend_type_name) = get_backend_type_names(proc_type);
    let backend_type = backend.backend_stream();

    quote! {
        type #backend_type_name = #backend_type;
        type #autodiff_backend_type_name = burn::backend::Autodiff<#backend_type_name>;
    }
}

pub(crate) fn get_backend_type(item: &ItemFn) -> Result<BackendType, Error> {
    let mut backends: Vec<BackendType> = Vec::new();

    #[cfg(feature = "wgpu")]
    backends.push(BackendType::Wgpu);
    #[cfg(feature = "tch")]
    backends.push(BackendType::Tch);
    #[cfg(feature = "ndarray")]
    backends.push(BackendType::Ndarray);

    match backends.len() {
        0 => Ok(DEFAULT_BACKEND.clone()),
        1 => Ok(backends.pop().expect("Should be able to pop one backend.")),
        _ => Err(Error::new(
            item.sig.ident.span(),
            "Only one backend can be enabled at a time. Please enable only one of `wgpu`, `tch` or `ndarray` features.",
        ))
    }
}