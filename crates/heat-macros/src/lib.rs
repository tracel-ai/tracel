use proc_macro::TokenStream;
use quote::quote;
use std::{
    collections::HashSet,
    sync::{Mutex, OnceLock},
};
use strum::Display;
use syn::{
    parse_macro_input, punctuated::Punctuated, spanned::Spanned, token::Comma, Error, ItemFn, Meta,
    Path,
};

#[derive(Eq, Hash, PartialEq, Display)]
#[strum(serialize_all = "snake_case")]
enum ProcedureType {
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

#[proc_macro_attribute]
pub fn heat(args: TokenStream, item: TokenStream) -> TokenStream {
    let mut errors = Vec::<Error>::new();
    let args: Punctuated<Meta, Comma> =
        parse_macro_input!(args with Punctuated::<Meta, syn::Token![,]>::parse_terminated);
    let item = parse_macro_input!(item as ItemFn);

    if args.len() != 1 {
        errors.push(Error::new(
            args.span(),
            "Expected one argument for the #[heat] attribute. Please provide `training`, `inference` or `setup`",
        ));
    }

    let fn_generics = &item.sig.generics;
    if fn_generics.params.len() != 1 {
        errors.push(Error::new(
            fn_generics.span(),
            "Expected exactly one generic parameter",
        ));
    }

    match fn_generics.params.first() {
        Some(syn::GenericParam::Type(_)) => {}
        _ => {
            errors.push(Error::new(
                fn_generics.span(),
                "Expected BackendType as a generic parameter",
            ));
        }
    }

    let fn_name = &item.sig.ident;

    #[allow(dead_code)]
    enum BackendType {
        Wgpu,
        Tch,
    }

    // --- Select backend type ---
    const DEFAULT_BACKEND: BackendType = BackendType::Wgpu;
    let backend = {
        let mut backends: Vec<BackendType> = Vec::new();

        #[cfg(feature = "wgpu")]
        backends.push(BackendType::Wgpu);
        #[cfg(feature = "tch")]
        backends.push(BackendType::Tch);

        if backends.len() > 1 {
            errors.push(Error::new(
                item.sig.ident.span(),
                "Only one backend can be enabled at a time",
            ));
        }

        let backend = {
            if backends.is_empty() {
                DEFAULT_BACKEND
            } else {
                backends.pop().unwrap()
            }
        };

        backend
    };
    // --- Select backend type ---

    let cfg_quote = {
        let mut cfg_quote = quote! {};
        match backend {
            BackendType::Wgpu => {
                cfg_quote = quote! {
                    #cfg_quote
                    type MyBackend = burn::backend::Wgpu<f32, i32>;
                    type MyAutodiffBackend = burn::backend::Autodiff<MyBackend>;
                    let device = burn::backend::wgpu::WgpuDevice::default();
                };
            }
            BackendType::Tch => {
                cfg_quote = quote! {
                    #cfg_quote
                    type MyBackend = burn::backend::libtorch::LibTorch<f32>;
                    type MyAutodiffBackend = burn::backend::Autodiff<MyBackend>;
                    let device = burn::backend::libtorch::LibTorchDevice::default();
                };
            }
        }
        cfg_quote
    };

    let heat_main = quote! {
        pub mod __heat_main {
            pub use crate::guide_mod::*;

        pub fn heat_main() {
            #cfg_quote

            fn heat_client(api_key: &str, url: &str, project: &str) -> tracel::heat::client::HeatClient {
                let creds = tracel::heat::client::HeatCredentials::new(api_key.to_owned());
                let client_config = tracel::heat::client::HeatClientConfig::builder(creds, project)
                    .with_endpoint(url)
                    .with_num_retries(10)
                    .build();
                tracel::heat::client::HeatClient::create(client_config)
                    .expect("Should connect to the Heat server and create a client")
            }

            let artifact_dir = "/tmp/guide";

            let mut client = heat_client("90902bd6-053a-4ae8-a51c-002898b549fb", "http://127.0.0.1:9001", "4dbca6a9-8245-4a8b-b954-83ef9ba459d1");

            let config = TrainingConfig::new(ModelConfig::new(10, 512), AdamConfig::new());

            client
            .start_experiment(&config)
            .expect("Experiment should be started");

            let res = #fn_name::<MyAutodiffBackend>(client.clone(), vec![device], config);

            match res {
                Ok(model) => {
                    client
                    .end_experiment_with_model::<MyAutodiffBackend, burn::record::HalfPrecisionSettings>(model.clone())
                    .expect("Experiment should end successfully");
                }
                Err(_) => {
                    client
                    .end_experiment_with_error("Error during training".to_string())
                    .expect("Experiment should end successfully");
                }
            }
        }}
    };

    // If there are any errors, combine them and return
    if !errors.is_empty() {
        let combined_error = errors
            .into_iter()
            .reduce(|mut acc, err| {
                acc.combine(err);
                acc
            })
            .unwrap();

        return combined_error.to_compile_error().into();
    }

    quote! {
        #item
        #heat_main
    }
    .into()
}

// #[cfg(feature = "test_ft")]
// fn test() {
//     compile_error!("This is a test feature");
// }
