use proc_macro::TokenStream;
use quote::{quote, quote_spanned};

use strum::Display;
use syn::{
    parse_macro_input, punctuated::Punctuated, spanned::Spanned, token::Comma, Error, ItemFn, Meta, Path,
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

    // name of the function
    let fn_name = &item.sig.ident;


    #[allow(dead_code)]
    enum BackendType {
        Wgpu,
        Tch,
        Ndarray,
    }

    // --- Select backend type ---
    const DEFAULT_BACKEND: BackendType = BackendType::Wgpu;
    let backend = {
        let mut backends: Vec<BackendType> = Vec::new();

        #[cfg(feature = "wgpu")]
        backends.push(BackendType::Wgpu);
        #[cfg(feature = "tch")]
        backends.push(BackendType::Tch);
        #[cfg(feature = "ndarray")]
        backends.push(BackendType::Ndarray);

        if backends.len() > 1 {
            errors.push(Error::new(
                item.sig.ident.span(),
                "Only one backend can be enabled at a time",
            ));
        }

        if backends.is_empty() {
            DEFAULT_BACKEND
        } else {
            backends.pop().unwrap()
        }
    };
    
    // --- Select backend type ---
    let backend_quote = {
        let mut backend_quote = quote! {};
        match backend {
            BackendType::Wgpu => {
                backend_quote = quote! {
                    #backend_quote
                    type MyBackend = burn::backend::Wgpu<f32, i32>;
                    type MyAutodiffBackend = burn::backend::Autodiff<MyBackend>;
                    let device = burn::backend::wgpu::WgpuDevice::default();
                };
            }
            BackendType::Tch => {
                backend_quote = quote! {
                    #backend_quote
                    type MyBackend = burn::backend::libtorch::LibTorch<f32>;
                    type MyAutodiffBackend = burn::backend::Autodiff<MyBackend>;
                    let device = burn::backend::libtorch::LibTorchDevice::default();
                };
            }
            BackendType::Ndarray => {
                backend_quote = quote! {
                    #backend_quote
                    type MyBackend = burn::backend::ndarray::NdArray<f32>;
                    type MyAutodiffBackend = burn::backend::Autodiff<MyBackend>;
                    let device = burn::backend::ndarray::NdArrayDevice::default();
                };
            }
        }
        backend_quote
    };

    let heat_main = quote! {
            pub use tracel::heat::run::*;

            pub fn heat_main() {
                #backend_quote

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

                let mut client = heat_client("dcaf7eb9-5acc-47d7-8b93-ca0fbb234096", "http://127.0.0.1:9001", "331a3907-bfd8-45e5-af54-1fee73a3c1b1");

                for config_path in get_run_config().configs_paths {
                    let config = burn::prelude::Config::load(config_path).expect("Config should be loaded");

                    client
                    .start_experiment(&config)
                    .expect("Experiment should be started");

                    let res = #fn_name::<MyAutodiffBackend>(client.clone(), vec![device.clone()], config);

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
                }
            }
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

#[proc_macro_attribute]
pub fn heat_cli_main(_args: TokenStream, item: TokenStream) -> TokenStream {
    let item = parse_macro_input!(item as ItemFn);

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

    let item = quote! {
        #item_sig {
            tracel::heat::cli::cli_main();
        }
    };

    quote! {
        #item
    }
    .into()
}
