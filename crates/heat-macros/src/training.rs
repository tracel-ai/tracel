use syn::{punctuated::Punctuated, spanned::Spanned, token::Comma, Error, ItemFn, Meta};

use crate::backend::*;
use crate::ProcedureType;
use quote::quote;

pub(crate) fn generate_training(
    _args: &Punctuated<Meta, Comma>,
    item: &ItemFn,
) -> Result<proc_macro2::TokenStream, Vec<Error>> {
    let mut errors = Vec::<Error>::new();

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
    };

    // Extract signature information
    let fn_name = &item.sig.ident;

    // Select backend
    let backend = match get_backend_type(item) {
        Ok(backend) => backend,
        Err(err) => {
            errors.push(err);
            return Err(errors);
        }
    };
    let backend_types = generate_backend_typedef_stream(&backend, &ProcedureType::Training);
    let (_, autodiff_backend_type) = get_backend_type_names(&ProcedureType::Training);
    let backend_default_device_quote = backend.default_device_stream();

    if !errors.is_empty() {
        return Err(errors);
    }

    Ok(quote! {
        #backend_types
        pub fn heat_training_main() -> Result<impl Module<#autodiff_backend_type>, tracel::heat::error::HeatSdkError> {
            let device = #backend_default_device_quote;

            fn heat_client(api_key: &str, url: &str, project: &str) -> tracel::heat::client::HeatClient {
                let creds = tracel::heat::client::HeatCredentials::new(api_key.to_owned());
                let client_config = tracel::heat::client::HeatClientConfig::builder(creds, project)
                    .with_endpoint(url)
                    .with_num_retries(10)
                    .build();
                tracel::heat::client::HeatClient::create(client_config)
                    .expect("Should connect to the Heat server and create a client")
            }
            let run_config = tracel::heat::run::get_run_config();

            let mut client = heat_client(&run_config.key, &run_config.heat_endpoint, &run_config.project);

            let config = burn::prelude::Config::load(run_config.config_path.clone()).expect("Config should be loaded");

            client
                .start_experiment(&config)
                .expect("Experiment should be started");

            let res = #fn_name::<#autodiff_backend_type>(client.clone(), vec![device.clone()], config);

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
