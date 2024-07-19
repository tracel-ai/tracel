use quote::quote;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::{generation::FileTree, print_err};

use super::cargo_toml::{CargoToml, Dependency, QueryType};

pub struct GeneratedCrate {
    name: String,
    cargo_toml: CargoToml,
    src: FileTree,
}

impl GeneratedCrate {
    pub fn new(name: String) -> Self {
        let mut cargo_toml = CargoToml::default();
        cargo_toml.set_package_name(name.clone());
        Self {
            name,
            cargo_toml,
            src: FileTree::Directory("src".to_string(), vec![]),
        }
    }

    pub fn src(&self) -> &FileTree {
        &self.src
    }

    pub fn src_mut(&mut self) -> &mut FileTree {
        &mut self.src
    }

    pub fn name(&self) -> String {
        self.name.clone()
    }

    pub fn add_dependency(&mut self, dependency: Dependency) {
        self.cargo_toml.add_dependency(dependency);
    }

    pub fn remove_dependency(&mut self, name: &str) {
        self.cargo_toml.remove_dependency(name);
    }

    pub fn set_package_name(&mut self, name: String) {
        self.name = name.clone();
        self.cargo_toml.set_package_name(name)
    }

    pub fn set_package_version(&mut self, version: String) {
        self.cargo_toml.set_package_version(version)
    }

    pub fn set_package_edition(&mut self, edition: String) {
        self.cargo_toml.set_package_edition(edition)
    }

    pub fn write_to(&self, path: &Path) -> Result<(), std::io::Error> {
        let cargo_toml_str = self.cargo_toml.to_string();

        let cargotoml_path = path.join("Cargo.toml");
        std::fs::write(cargotoml_path, cargo_toml_str)
            .expect("Should be able to write Cargo.toml file.");

        self.src.write_to(path)
    }

    pub fn into_file_tree(self) -> FileTree {
        FileTree::new_dir(
            self.name.clone(),
            [
                FileTree::new_file("Cargo.toml", self.cargo_toml.to_string()),
                self.src,
            ],
        )
    }
}

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

fn get_cargo_dependency(package: &MetadataDependency) -> Dependency {
    let version = package.req.clone();

    let is_local = package.path.is_some();
    if is_local {
        Dependency::new_path(
            package.name.clone(),
            version,
            package.path.as_ref().unwrap().clone(),
            vec![],
        )
    } else {
        let source = package.source.as_ref().unwrap();
        let source_kind = {
            if source.starts_with("git") {
                "git"
            } else if source.starts_with("registry") {
                "registry"
            } else {
                "other"
            }
        };

        let source = source
            .as_str()
            .strip_prefix(&format!("{}+", source_kind))
            .expect("Should be able to strip prefix.");
        let url = url::Url::parse(source).expect("Should be able to parse url.");

        let dep_url = format!(
            "{}://{}{}",
            url.scheme(),
            url.host_str().expect("Should be able to get host"),
            url.path()
        );

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

                Dependency::new_git(package.name.clone(), version, dep_url, query_type, vec![])
            }
            "registry" => Dependency::new(package.name.clone(), version, vec![]),
            _ => {
                panic!("Error")
            }
        }
    }
}

fn find_required_dependencies(req_deps: Vec<&str>) -> Vec<Dependency> {
    let manifest_cmd = std::process::Command::new("cargo")
        .arg("metadata")
        .arg("--no-deps")
        .args(["--format-version", "1"])
        .output()
        .expect("Should be able to run cargo init.");

    let manifest_str =
        std::str::from_utf8(&manifest_cmd.stdout).expect("Should be able to parse stdout.");
    let manifest_json: serde_json::Value =
        serde_json::from_str(manifest_str).expect("Should be able to parse json.");
    let packages_array = manifest_json["packages"]
        .as_array()
        .expect("Should be able to get workspace members.");
    let our_package_name =
        std::env::var("CARGO_PKG_NAME").expect("Should be able to get package name.");
    let our_package = packages_array
        .iter()
        .find(|package| package["name"] == our_package_name)
        .expect("Should be able to find our package.");
    let our_package_dependencies = our_package["dependencies"]
        .as_array()
        .expect("Should be able to get dependencies.");

    let req_deps_metadata: Vec<MetadataDependency> = our_package_dependencies
        .iter()
        .filter(|dep| {
            req_deps.contains(&dep["name"].as_str().expect("Should be able to get name."))
        })
        .map(|dep| {
            serde_json::from_value(dep.clone()).expect("Should be able to parse dep metadata.")
        })
        .collect();

    req_deps_metadata.iter().map(get_cargo_dependency).collect()
}

fn generate_clap_cli() -> proc_macro2::TokenStream {
    quote! {
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
            }
            Err(_) => {
                train_cmd_context.client()
                .end_experiment_with_error("Error during training".to_string())
                .expect("Experiment should end successfully");
            }
        }
    }
}

fn generate_proc_call(
    item: syn::ItemFn,
    mod_path: &str,
    fn_name: &str,
) -> proc_macro2::TokenStream {
    let syn_func_path = syn::parse_str::<syn::Path>(&format!("{}::{}", mod_path, fn_name))
        .expect("Failed to parse path.");

    quote! {
        let res = trigger(#syn_func_path, train_cmd_context.clone());
        res
    }
}

fn generate_main_rs(main_backend: &str) -> String {
    let flags = crate::registry::get_flags();

    let train_match_arms: Vec<proc_macro2::TokenStream> = flags
        .iter()
        .filter(|flag| flag.proc_type == "training")
        .map(|flag| {
            let item_fn =
                syn_serde::json::from_slice(flag.token_stream).expect("Failed to parse item fn.");
            let proc_call = generate_proc_call(item_fn, flag.mod_path, flag.fn_name);

            let fn_name = flag.fn_name;

            quote! {
                 #fn_name => {
                    #proc_call
                }
            }
        })
        .collect();

    let train_func_match = quote! {
        match func.as_str() {
            #(#train_match_arms)*
            _ => panic!("Unknown training function: {}", func),
        }
    };

    let backend = match crate::generation::crate_gen::backend::get_backend_type(main_backend) {
        Ok(backend) => backend,
        Err(err) => {
            print_err!("{}", err);
            std::process::exit(1);
        }
    };

    let backend_types =
        crate::generation::crate_gen::backend::generate_backend_typedef_stream(&backend);
    let (_backend_type_name, autodiff_backend_type_name) =
        crate::generation::crate_gen::backend::get_backend_type_names();
    let backend_default_device = backend.default_device_stream();

    let clap_cli = generate_clap_cli();
    let generated_training =
        generate_training_function(&train_func_match, &autodiff_backend_type_name);

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

pub fn create_crate(
    crate_name: &str,
    user_project_name: &str,
    user_project_dir: &str,
    burn_features: Vec<&str>,
    backend: &str,
) -> GeneratedCrate {
    // Create the generated crate package
    let mut generated_crate = GeneratedCrate::new(crate_name.to_string());
    generated_crate.set_package_edition("2021".to_string());
    generated_crate.set_package_version("0.0.0".to_string());

    // Add dependencies
    generated_crate.add_dependency(Dependency::new_path(
        user_project_name.to_string(),
        "*".to_string(),
        user_project_dir.to_string(),
        vec![],
    ));
    generated_crate.add_dependency(Dependency::new(
        "clap".to_string(),
        "*".to_string(),
        vec!["cargo".to_string()],
    ));
    find_required_dependencies(vec!["tracel", "burn"])
        .drain(..)
        .for_each(|mut dep| {
            if dep.name == "burn" {
                burn_features.iter().for_each(|feature| {
                    dep.add_feature(feature.to_string());
                });
            }
            if dep.name == "tracel" {
                dep.add_feature("heat-sdk".to_string());
            }
            generated_crate.add_dependency(dep);
        });

    // Generate source files
    generated_crate
        .src_mut()
        .insert(FileTree::new_file("main.rs", generate_main_rs(backend)));

    generated_crate
}