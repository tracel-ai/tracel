pub mod backend;
mod cargo_toml;

use quote::quote;
use serde::{Deserialize, Serialize};
use std::hash::{Hash, Hasher};

use crate::generation::FileTree;

use super::backend::BackendType;
use crate::burn_dir::BurnDir;
use crate::burn_dir::cache::CacheState;
use cargo_toml::{CargoToml, Dependency, QueryType};

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

    pub fn src_mut(&mut self) -> &mut FileTree {
        &mut self.src
    }

    pub fn name(&self) -> String {
        self.name.clone()
    }

    pub fn add_dependency(&mut self, dependency: Dependency) {
        self.cargo_toml.add_dependency(dependency);
    }

    pub fn set_package_version(&mut self, version: String) {
        self.cargo_toml.set_package_version(version)
    }

    pub fn set_package_edition(&mut self, edition: String) {
        self.cargo_toml.set_package_edition(edition)
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

    pub fn compute_hash(&self) -> String {
        self.cargo_toml
            .to_string()
            .as_bytes()
            .iter()
            .fold(0u64, |acc, &b| acc.wrapping_add(b as u64))
            .to_string()
    }

    pub fn write_to_burn_dir(
        self,
        burn_dir: &BurnDir,
        cache: &mut CacheState,
    ) -> std::io::Result<()> {
        let name = self.name.to_owned();
        let file_tree = self.into_file_tree();
        let mut hasher = std::hash::DefaultHasher::new();
        file_tree.hash(&mut hasher);
        let file_tree_hash = hasher.finish();

        if let Some(cached_crate) = cache.get_crate(&name) {
            if cached_crate.hash == file_tree_hash {
                return Ok(());
            } else {
                std::fs::remove_dir_all(burn_dir.crates_dir().join(&name))?;
                cache.remove_crate(&name);
            }
        }

        let burn_dir_path = burn_dir.crates_dir().join(&name);

        std::fs::create_dir_all(&burn_dir_path)?;
        file_tree.write_to(burn_dir_path.as_path())?;

        cache.add_crate(
            &name,
            burn_dir_path.to_string_lossy().to_string(),
            file_tree_hash,
        );

        Ok(())
    }
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

                let dep_url = format!(
                    "{}://{}{}",
                    url.scheme(),
                    url.host_str().expect("Should be able to get host"),
                    url.path()
                );

                Dependency::new_git(package.name.clone(), version, dep_url, query_type, vec![])
            }
            "registry" => Dependency::new(
                package.name.clone(),
                version,
                package.registry.clone(),
                vec![],
            ),
            _ => {
                panic!("Error")
            }
        }
    }
}

fn find_required_dependencies(req_deps: Vec<&str>) -> Vec<Dependency> {
    // TODO: Refactor to use cargo metadata crate instead of reading manually.
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
                    .help("The project path")
                    .required(true),
                clap::Arg::new("key")
                    .short('k')
                    .long("key")
                    .help("The API key")
                    .required(true),
                clap::Arg::new("api-endpoint")
                    .short('e')
                    .long("api-endpoint")
                    .help("The Burn Central endpoint")
                    .required(true),
                clap::Arg::new("wss")
                    .short('w')
                    .long("wss")
                    .help("Whether to use WSS")
                    .required(true),
            ]);

            command
        }
    }
}

fn generate_training_function(
    train_func_match: &proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    quote! {
        let training_config_str = std::fs::read_to_string(&config_path).expect("Config should be read");
        let training_config: serde_json::Value = serde_json::from_str(&training_config_str).expect("Config should be deserialized");

        let mut train_cmd_context = TrainCommandContext::new(client, vec![device], training_config_str);

        train_cmd_context.client()
            .start_experiment(&training_config)
            .expect("Experiment should be started");

        pub fn trigger<B: Backend, T, M: Module<B>, E: Into<Box<dyn std::error::Error>>, H: TrainCommandHandler<B, T, M, E>>(handler: H, context: TrainCommandContext<B>) -> Result<M, Box<dyn std::error::Error>> {
            match handler.call(context) {
                Ok(model) => Ok(model),
                Err(e) => Err(e.into()),
            }
        }

        #train_func_match;
    }
}

fn generate_proc_call(
    _item: syn::ItemFn,
    mod_path: &str,
    fn_name: &str,
) -> proc_macro2::TokenStream {
    let syn_func_path = syn::parse_str::<syn::Path>(&format!("{}::{}", mod_path, fn_name))
        .expect("Failed to parse path.");

    quote! {
        trigger(#syn_func_path, train_cmd_context.clone())
    }
}

fn generate_main_rs(main_backend: &BackendType) -> String {
    let flags = crate::registry::get_flags();

    let backend_types =
        crate::generation::crate_gen::backend::generate_backend_typedef_stream(main_backend);
    let (_backend_type_name, autodiff_backend_type_name) =
        crate::generation::crate_gen::backend::get_backend_type_names();
    let backend_default_device = main_backend.default_device_stream();

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
                    match #proc_call {
                        Ok(model) => {
                            train_cmd_context.client()
                            .end_experiment_with_model::<#autodiff_backend_type_name, burn::record::HalfPrecisionSettings>(model.clone())
                            .expect("Experiment should end successfully");
                        }
                        Err(e) => {
                            train_cmd_context.client()
                            .end_experiment_with_error(e.to_string())
                            .expect("Experiment should end successfully");
                        }
                    }
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

    let clap_cli = generate_clap_cli();
    let generated_training = generate_training_function(&train_func_match);

    let bin_content: proc_macro2::TokenStream = quote! {
        #backend_types
        #clap_cli

        use burn_central::command::train::*;
        use burn::prelude::*;

        fn create_client(api_key: &str, url: &str, project: &str, wss: bool) -> burn_central::client::BurnCentralClient {
            let creds = burn_central::client::BurnCentralCredentials::new(api_key.to_owned());
            let client_config = burn_central::client::BurnCentralClientConfig::builder(creds, burn_central::schemas::ProjectPath::try_from(project.to_string()).expect("Project path should be valid."))
                .with_endpoint(url)
                .with_wss(wss)
                .with_num_retries(10)
                .build();
            burn_central::client::BurnCentralClient::create(client_config)
                .expect("Should connect to the server and create a client")
        }

        fn main() {
            let matches = generate_clap().get_matches();

            let device = #backend_default_device;

            let key = matches.get_one::<String>("key").expect("key should be set.");
            let endpoint = matches.get_one::<String>("api-endpoint").expect("api-endpoint should be set.");
            let project = matches.get_one::<String>("project").expect("project should be set.");
            let wss = matches.get_one::<String>("wss").expect("wss should be set.").parse::<bool>().expect("wss should be a boolean.");

            let client = create_client(&key, &endpoint, &project, wss);

            if let Some(train_matches) = matches.subcommand_matches("train") {
                let func = train_matches.get_one::<String>("func").expect("func should be set.");
                let config_path = train_matches.get_one::<String>("config").expect("config should be set.");

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
    backend: &BackendType,
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
        None,
        vec!["cargo".to_string()],
    ));
    generated_crate.add_dependency(Dependency::new(
        "serde_json".to_string(),
        "*".to_string(),
        None,
        vec![],
    ));
    find_required_dependencies(vec!["burn-central", "burn"])
        .drain(..)
        .for_each(|mut dep| {
            if dep.name == "burn" {
                burn_features.iter().for_each(|feature| {
                    dep.add_feature(feature.to_string());
                });
            }
            if dep.name == "burn-central" {
                dep.add_feature("burn-central-client".to_string());
            }
            generated_crate.add_dependency(dep);
        });

    // Generate source files
    generated_crate
        .src_mut()
        .insert(FileTree::new_file("main.rs", generate_main_rs(backend)));

    generated_crate
}
