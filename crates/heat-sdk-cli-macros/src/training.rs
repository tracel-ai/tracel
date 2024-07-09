use syn::{parse_quote, GenericArgument, Ident, PathArguments, ReturnType, Type, TypePath};
use syn::{punctuated::Punctuated, spanned::Spanned, token::Comma, Error, ItemFn, Meta};

use crate::backend::*;
use crate::ProcedureType;
use quote::quote;

fn check_training_module_type(item: &ItemFn) -> Result<(), Error> {
    if let ReturnType::Type(_, type_box) = &item.sig.output {
        if let Type::Path(TypePath { path, .. }) = &**type_box {
            if path.segments.last().unwrap().ident == "Result" {
                if let PathArguments::AngleBracketed(angle_bracketed) =
                    &path.segments.last().unwrap().arguments
                {
                    let args: Vec<_> = angle_bracketed.args.iter().collect();
                    if let GenericArgument::Type(Type::Path(_)) = args[0] {
                        return Ok(());
                    }
                }
            }
        }
    }

    Err(Error::new(
        item.sig.output.span(),
        "Expected return type to be Result<M<B>, _> where M<B> is the module type with the backend B.",
    ))
}

/// Returns the module type used for the training function injected with the backend of the function.
fn get_training_module_type(item: &ItemFn, new_backend: &Ident) -> Option<GenericArgument> {
    if let ReturnType::Type(_, type_box) = &item.sig.output {
        if let Type::Path(TypePath { path, .. }) = &**type_box {
            if path.segments.last().unwrap().ident == "Result" {
                if let PathArguments::AngleBracketed(angle_bracketed) =
                    &path.segments.last().unwrap().arguments
                {
                    let args: Vec<_> = angle_bracketed.args.iter().collect();
                    if let GenericArgument::Type(Type::Path(type_path)) = args[0] {
                        let model_type = &type_path.path.segments.last().unwrap().ident;
                        let new_model_type: syn::Type = parse_quote! { #model_type<#new_backend> };
                        let new_result_type = GenericArgument::Type(new_model_type);
                        return Some(new_result_type);
                    }
                }
            }
        }
    }

    None
}

pub(crate) fn generate_training(
    _args: &Punctuated<Meta, Comma>,
    item: &ItemFn,
    generating_cli: bool,
    project_dir: &str,
) -> Result<proc_macro2::TokenStream, Vec<Error>> {
    let mut errors = Vec::<Error>::new();

    // Extract signature information
    let fn_name = &item.sig.ident;
    let fn_generics = &item.sig.generics;

    // Enforce backend generic (should be exactly one generic parameter named `B` for the backend type)
    if let Err(err) = enforce_fn_backend_generic(fn_generics) {
        errors.push(err);
    }

    // Generate return type of the function
    if let Err(e) = check_training_module_type(item) {
        errors.push(e);
    }

    if !errors.is_empty() {
        return Err(errors);
    }

    if !generating_cli {
        return Ok(quote! {});
    }

    let mut errors = Vec::<Error>::new();

    let metadata = crate::metadata::load_metadata(&format!(
        "{}/.heat/crates/heat-sdk-cli/run_metadata.toml",
        project_dir
    ))
    .expect("Should be able to load metadata file.");

    // Select backend
    let backend = match get_backend_type(item, &metadata.options.backend) {
        Ok(backend) => backend,
        Err(err) => {
            errors.push(err);
            return Err(errors);
        }
    };

    let backend_types =
        generate_backend_typedef_stream(&backend, &ProcedureType::Training, &fn_name.to_string());
    let (_, autodiff_backend_type) =
        get_backend_type_names(&ProcedureType::Training, &fn_name.to_string());
    let backend_default_device_quote = backend.default_device_stream();

    let training_module_type = get_training_module_type(item, &autodiff_backend_type);
    if training_module_type.is_none() {
        errors.push(Error::new(
            item.sig.output.span(),
            "Expected return type to be Result<M<B>, _> where M<B> is the module type with the backend B.",
        ));
    }

    if !errors.is_empty() {
        return Err(errors);
    }

    let rand_symbol = syn::Ident::new(
        &format!("__{}", &uuid::Uuid::new_v4().as_simple().to_string()),
        proc_macro2::Span::call_site(),
    );

    let test_json_path = format!(
        "{}/.heat/crates/heat-sdk-cli/run_metadata.toml",
        project_dir
    );
    let test_json_path: syn::LitStr =
        syn::LitStr::new(&test_json_path, proc_macro2::Span::call_site());
    let rebuild_trigger = quote! {
        const #rand_symbol: &[u8] = include_bytes!(#test_json_path);
    };
    let training_main_name = syn::Ident::new(
        &format!("heat_training_main_{}", fn_name),
        proc_macro2::Span::call_site(),
    );

    Ok(quote! {
        #rebuild_trigger
        #backend_types
        pub fn #training_main_name(config_path: String, key: String, project: String, heat_endpoint: String) -> Result<#training_module_type, tracel::heat::error::HeatSdkError> {
            fn create_heat_client(api_key: &str, url: &str, project: &str) -> tracel::heat::client::HeatClient {
                let creds = tracel::heat::client::HeatCredentials::new(api_key.to_owned());
                let client_config = tracel::heat::client::HeatClientConfig::builder(creds, project)
                    .with_endpoint(url)
                    .with_num_retries(10)
                    .build();
                tracel::heat::client::HeatClient::create(client_config)
                    .expect("Should connect to the Heat server and create a client")
            }

            let device = #backend_default_device_quote;

            let mut client = create_heat_client(&key, &heat_endpoint, &project);
            let training_config = burn::prelude::Config::load(config_path).expect("Config should be loaded");

            client
                .start_experiment(&training_config)
                .expect("Experiment should be started");

            let res = #fn_name::<#autodiff_backend_type>(client.clone(), vec![device.clone()], training_config);

            match res {
                Ok(model) => {
                    client
                    .end_experiment_with_model::<#autodiff_backend_type, burn::record::HalfPrecisionSettings>(model.clone())
                    .expect("Experiment should end successfully");
                    Ok(model)
                }
                Err(_) => {
                    client
                    .end_experiment_with_error("Error during training".to_string())
                    .expect("Experiment should end successfully");
                    Err(tracel::heat::error::HeatSdkError::MacroError("Error during training".to_string()))
                }
            }
        }
    })
}
