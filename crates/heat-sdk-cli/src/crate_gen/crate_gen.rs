use quote::quote;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::print_err;

pub fn get_heat_dir() -> PathBuf {
    PathBuf::from(".heat")
}

#[derive(Debug, Deserialize, Serialize)]
pub struct MetadataDependency {
    pub name: String,
    pub source: Option<String>,
    pub req: String,
    pub kind: Option<String>,
    pub rename: Option<String>,
    pub optional: bool,
    pub uses_default_features: bool,
    pub features: Vec<String>,
    pub target: Option<String>,
    pub path: Option<String>,
    pub registry: Option<String>,
}

fn get_cargo_toml_dependency(package: &MetadataDependency) -> toml_edit::Item {
    let mut dep = toml_edit::table();
    let is_local = package.path.is_some();
    dep["version"] = toml_edit::value(&package.req);
    if is_local {
        dep["path"] = toml_edit::value(package.path.as_ref().unwrap());
    } else {
        if let Some(source) = &package.source {
            let source_kind = {
                if source.starts_with("git") {
                    "git"
                } else if source.starts_with("registry") {
                    "registry"
                } else {
                    "other"
                }
            };

            let source = source.as_str().strip_prefix(&format!("{}+", source_kind)).expect("Should be able to strip prefix.");
            let url = url::Url::parse(source).expect("Should be able to parse url.");

            let dep_url = format!("{}://{}{}", url.scheme(), url.host_str().expect("Should be able to get host"), url.path());

            enum QueryType {
                Branch(String),
                Tag(String),
                Rev(String),
            }

            match source_kind {
                "git" => {
                    let query = url.query();
                    let query_type = match query {
                        Some(q) => {
                            let parts: Vec<&str> = q.split('=').collect();
                            match parts[0] {
                                "branch" => QueryType::Branch(parts[1].to_string()),
                                "tag" => QueryType::Tag(parts[1].to_string()),
                                "rev" => QueryType::Rev(parts[1].to_string()),
                                _ => panic!("Error"),
                            }
                        }
                        None => QueryType::Branch("master".to_string()),
                    };

                    match query_type {
                        QueryType::Branch(branch) => {
                            dep["branch"] = toml_edit::value(branch);
                        }
                        QueryType::Tag(tag) => {
                            dep["tag"] = toml_edit::value(tag);
                        }
                        QueryType::Rev(rev) => {
                            dep["rev"] = toml_edit::value(rev);
                        }
                    }

                    dep["git"] = toml_edit::value(dep_url);
                }
                "registry" => {}
                _ => { panic!("Error") }
            }
        }
    }
    
    dep
}

fn generate_cargo_toml(project_name: &str, project_dir: &str, burn_features: Vec<&str>) -> String {
    

    let mut cargo_toml = toml_edit::DocumentMut::new();
    // package settings
    let mut package = toml_edit::Table::new();
    package.insert("edition", toml_edit::value("2021"));
    package.insert("version", toml_edit::value("0.1.0"));
    package.insert("name", toml_edit::value("generated_heat_crate"));

    // dependencies
    let mut dependencies = toml_edit::Table::new();
    dependencies[project_name]["path"] = toml_edit::value(project_dir);
    if let Some(t) = dependencies[project_name].as_inline_table_mut() {
        t.fmt()
    }

    dependencies["clap"]["version"] = toml_edit::value("*");

    dependencies["clap"]["features"] = toml_edit::Item::Value(toml_edit::Value::Array(toml_edit::Array::new()));
    if let Some(a) = dependencies["clap"]["features"].as_array_mut() {
        a.push("cargo");
    }

    let manifest_cmd = std::process::Command::new("cargo")
        .arg("metadata")
        .arg("--no-deps")
        .args(["--format-version", "1"])
        .args(["--manifest-path", &format!("{}/Cargo.toml", &project_dir)])
        .current_dir(&project_dir)
        .output()
        .expect("Should be able to run cargo init.");

    let our_package_name = std::env::var("CARGO_PKG_NAME").expect("Should be able to get package name.");

    let manifest_str = std::str::from_utf8(&manifest_cmd.stdout).expect("Should be able to parse stdout.");
    let manifest_json: serde_json::Value = serde_json::from_str(manifest_str).expect("Should be able to parse json.");
    let packages_array = manifest_json["packages"].as_array().expect("Should be able to get workspace members.");
    let our_package = packages_array.iter().find(|package| package["name"] == our_package_name).expect("Should be able to find our package.");
    let our_package_dependencies = our_package["dependencies"].as_array().expect("Should be able to get dependencies.");

    let tracel_dep = our_package_dependencies.iter().find(|dep| dep["name"] == "tracel").expect("Should be able to find tracel dependency.");
    let burn_dep = our_package_dependencies.iter().find(|dep| dep["name"] == "burn").expect("Should be able to find burn dependency.");

    let tracel_dep_metadata = serde_json::from_value::<MetadataDependency>(tracel_dep.clone()).expect("Should be able to parse tracel dep metadata.");
    let burn_dep_metadata = serde_json::from_value::<MetadataDependency>(burn_dep.clone()).expect("Should be able to parse burn dep metadata.");

    dependencies["tracel"] = get_cargo_toml_dependency(&tracel_dep_metadata);
    if let Some(t) = dependencies["tracel"].as_inline_table_mut() {
        t.fmt()
    }
    dependencies["tracel"]["features"] = toml_edit::Item::Value(toml_edit::Value::Array(toml_edit::Array::new()));
    if let Some(a) = dependencies["tracel"]["features"].as_array_mut() {
        a.push("heat-sdk");
        
    }

    dependencies["burn"] = get_cargo_toml_dependency(&burn_dep_metadata);
    if let Some(t) = dependencies["burn"].as_inline_table_mut() {
        t.fmt()
    }
    dependencies["burn"]["features"] = toml_edit::Item::Value(toml_edit::Value::Array(toml_edit::Array::new()));
    if let Some(a) = dependencies["burn"]["features"].as_array_mut() {
        a.extend(burn_features);
    }

    // workspace
    let workspace = toml_edit::table();

    // insert into cargo_toml
    cargo_toml.insert("package", toml_edit::Item::Table(package));
    cargo_toml.insert("dependencies", toml_edit::Item::Table(dependencies));
    cargo_toml.insert("workspace", workspace);
    cargo_toml.to_string()
}

fn generate_clap_cli() -> proc_macro2::TokenStream
{
    quote!{
        fn generate_clap() -> clap::Command
        {
            let train_command = clap::command!()
            .name("train")
            .about("Train a model.")
            .arg(clap::Arg::new("func")
                .help("The training function to use.")
                .required(true)
                .index(1)
            )
            .arg(clap::Arg::new("config")
                .short('c')
                .long("config")
                .help("The training configuration to use.")
                .required(true)
                .index(2)
            );

        let infer_command = clap::command!()
            .name("infer")
            .about("Infer using a model.")
            .arg(clap::Arg::new("func")
                .help("The inference function to use.")
                .required(true)
                .index(1)
            )
            .arg(clap::Arg::new("model")
                .short('m')
                .long("model")
                .help("The model to use for inference.")
                .required(true)
                .index(2)
            );

        let command = clap::command!()
            .subcommands(
                vec![
                    train_command,
                    infer_command
                ]
            )
            .args([
                clap::Arg::new("project")
                    .short('p')
                    .long("project")
                    .help("The project ID")
                    .required(true),
                clap::Arg::new("key")
                    .short('k')
                    .long("key")
                    .help("The API key")
                    .required(true),
                clap::Arg::new("heat-endpoint")
                    .short('e')
                    .long("heat-endpoint")
                    .help("The Heat endpoint")
                    .default_value("http://127.0.0.1:9001"),
            ]);

            command
        } 
    }
}

fn generate_training_function(
    train_func_match: &proc_macro2::TokenStream,
    autodiff_backend: &proc_macro2::Ident,
) -> proc_macro2::TokenStream {
    quote! {    
        let mut client = create_heat_client(&key, &heat_endpoint, &project);
        let training_config_str = std::fs::read_to_string(&config_path).expect("Config should be read");

        let mut train_cmd_context = TrainCommandContext::new(client, vec![device], training_config_str);

        let conf_ser = train_cmd_context.config().as_bytes().to_vec();
        train_cmd_context.client()
            .start_experiment(&conf_ser)
            .expect("Experiment should be started");

        pub fn trigger<D, B: Backend, T, M: Module<B>, H: TrainCommandHandler<D, B, T, M>>(handler: H, context: TrainCommandContext<D>) -> TrainResult<M> {
            handler.call(context)
        }

        let res = #train_func_match;

        match res {
            Ok(model) => {
                train_cmd_context.client()
                .end_experiment_with_model::<#autodiff_backend, burn::record::HalfPrecisionSettings>(model.clone())
                .expect("Experiment should end successfully");
                // Ok(model)
            }
            Err(_) => {
                train_cmd_context.client()
                .end_experiment_with_error("Error during training".to_string())
                .expect("Experiment should end successfully");
                // Err(tracel::heat::error::HeatSdkError::MacroError("Error during training".to_string()))
            }
        }
    }
}


fn generate_proc_call(item: syn::ItemFn, mod_path: &str, fn_name: &str) -> proc_macro2::TokenStream {
    let syn_func_path = syn::parse_str::<syn::Path>(&format!("{}::{}", mod_path, fn_name)).expect("Failed to parse path.");

    quote! {        
        let res = trigger(#syn_func_path, train_cmd_context.clone());
        res
    }
}

fn generate_main_rs(main_backend: &str) -> String {
    let flags = crate::registry::get_flags();

    let train_match_arms: Vec<proc_macro2::TokenStream> = flags.iter().filter(|flag| flag.proc_type == "training").map(|flag| {
        let item_fn = syn_serde::json::from_slice(&flag.token_stream).expect("Failed to parse item fn.");
        let proc_call = generate_proc_call(item_fn, flag.mod_path, flag.fn_name);

        let fn_name = flag.fn_name;

        quote! {
             #fn_name => {
                // #syn_func_path(config_path.to_string(), key.to_string(), project.to_string(), heat_endpoint.to_string()).expect(&format!("Should be able to run training function. {}", #flag_name))
                #proc_call
            }
        }
    }).collect();

    let train_func_match = quote! {
        match func.as_str() {
            #(#train_match_arms)*
            _ => panic!("Unknown training function: {}", func),
        }
    };

    let backend = match crate::crate_gen::backend::get_backend_type(main_backend) {
        Ok(backend) => backend,
        Err(err) => {
            print_err!("{}", err);
            std::process::exit(1);
        }
    };

    let backend_types =
    crate::crate_gen::backend::generate_backend_typedef_stream(&backend);
    let (_backend_type_name, autodiff_backend_type_name) =
    crate::crate_gen::backend::get_backend_type_names();
    let backend_default_device = backend.default_device_stream();

    let clap_cli = generate_clap_cli();
    let generated_training = generate_training_function(&train_func_match,&autodiff_backend_type_name);


    let bin_content: proc_macro2::TokenStream = quote! {
        #backend_types
        #clap_cli

        use tracel::heat::command::train::*;
        use burn::prelude::*;

        fn create_heat_client(api_key: &str, url: &str, project: &str) -> tracel::heat::client::HeatClient {
            let creds = tracel::heat::client::HeatCredentials::new(api_key.to_owned());
            let client_config = tracel::heat::client::HeatClientConfig::builder(creds, project)
                .with_endpoint(url)
                .with_num_retries(10)
                .build();
            tracel::heat::client::HeatClient::create(client_config)
                .expect("Should connect to the Heat server and create a client")
        }

        fn main() {
            let matches = generate_clap().get_matches();

            let device = #backend_default_device;

            if let Some(train_matches) = matches.subcommand_matches("train") {
                let func = train_matches.get_one::<String>("func").expect("func should be set.");
                let config_path = train_matches.get_one::<String>("config").expect("config should be set.");
                let project = matches.get_one::<String>("project").expect("project should be set.");
                let key = matches.get_one::<String>("key").expect("key should be set.");
                let heat_endpoint = matches.get_one::<String>("heat-endpoint").expect("heat-endpoint should be set.");

                #generated_training
            }
            else if let Some(infer_matches) = matches.subcommand_matches("infer") {
                let _func = infer_matches.get_one::<String>("func").expect("func should be set.");
                let _model = infer_matches.get_one::<String>("model").expect("model should be set.");
            }
            else {
                panic!("Should have a train|infer subcommand.");
            }
        }
    };

    let syn_tree = syn::parse2(bin_content).expect("Failed to parse bin content");
    prettyplease::unparse(&syn_tree).to_string()
}

pub fn create_crate(burn_features: Vec<&str>, backend: &str) {
    let project_dir =
        std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR should be set.");
    let project_name = std::env::var("CARGO_PKG_NAME").expect("CARGO_PKG_NAME should be set.");

    let mut crate_path = PathBuf::from(project_dir.clone());

    crate_path.push(get_heat_dir());
    std::fs::create_dir_all(&crate_path).expect("Should be able to create crate directory.");

    std::fs::write(crate_path.join(".gitignore"), "*")
        .expect("Should be able to write gitignore file.");

    crate_path.extend(["crates", "generated-heat-sdk-crate"]);
    std::fs::create_dir_all(&crate_path).expect("Should be able to create crate directory.");

    // src + src/main.rs
    let mut main_path = crate_path.join("src");
    std::fs::create_dir_all(&main_path).expect("Should be able to create src directory.");
    main_path.push("main.rs");

    // generate and paste new code into main.rs if content has changed since last run
    let last_bin_content = std::fs::read_to_string(&main_path);
    let new_bin_content = generate_main_rs(backend);

    let should_write = match last_bin_content {
        Ok(ref content) => content != &new_bin_content,
        Err(_) => true,
    };

    // todo hash comparison
    if should_write {
        if let Err(e) = std::fs::write(&main_path, &new_bin_content) {
            eprintln!("Failed to write bin file: {}", e);
        }
    }

    let cargo_toml_str = generate_cargo_toml(&project_name, &project_dir, burn_features);

    let cargotoml_path = crate_path.join("Cargo.toml");
    std::fs::write(cargotoml_path, cargo_toml_str)
        .expect("Should be able to write Cargo.toml file.");
}