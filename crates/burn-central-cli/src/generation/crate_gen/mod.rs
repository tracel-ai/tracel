pub mod backend;
mod cargo_toml;

use crate::{
    entity::projects::burn_dir::{BurnDir, cache::CacheState},
    tools::function_discovery::FunctionMetadata,
};
use quote::quote;
use std::hash::{Hash, Hasher};

use crate::generation::{FileTree, crate_gen::cargo_toml::FeatureFlag};

use super::backend::BackendType;
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

    #[allow(dead_code)]
    pub fn name(&self) -> String {
        self.name.clone()
    }

    pub fn add_dependency(&mut self, dependency: Dependency) {
        self.cargo_toml.add_dependency(dependency);
    }

    #[allow(dead_code)]
    pub fn add_feature(&mut self, feature: &str, deps: &[impl ToString]) {
        self.cargo_toml.add_feature(FeatureFlag {
            name: feature.to_string(),
            deps: deps.iter().map(|dep| dep.to_string()).collect(),
        });
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

    pub fn write_to_burn_dir(
        self,
        burn_dir: &BurnDir,
        cache: &mut CacheState,
    ) -> std::io::Result<()> {
        let name = self.name.to_owned();
        let file_tree = self.into_file_tree();
        let mut hasher = std::hash::DefaultHasher::new();
        file_tree.hash(&mut hasher);
        let file_tree_hash = hasher.finish().to_string();

        if let Some(cached_crate) = cache.get_crate(&name) {
            if cached_crate.hash == file_tree_hash {
                return Ok(());
            } else {
                cache.remove_crate(&name);
            }
        }

        let burn_dir_path = burn_dir.crates_dir().join(&name);

        std::fs::create_dir_all(&burn_dir_path)?;
        file_tree.write_to(burn_dir_path.parent().unwrap())?;

        cache.add_crate(
            &name,
            burn_dir_path.to_string_lossy().to_string(),
            file_tree_hash,
        );

        Ok(())
    }
}

fn get_cargo_dependency(package: &cargo_metadata::Dependency) -> Dependency {
    let version = package.req.to_string();

    let is_local = package.path.is_some();
    if is_local {
        Dependency::new_path(
            package.name.to_string(),
            version,
            package.path.as_ref().unwrap().to_string(),
            vec![],
        )
    } else {
        let source = package.source.as_ref().unwrap().to_string();
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
            .strip_prefix(&format!("{source_kind}+"))
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

fn find_required_dependencies(
    current_pkg: &cargo_metadata::Package,
    req_deps: Vec<&str>,
) -> Vec<Dependency> {
    current_pkg
        .dependencies
        .iter()
        .filter(|dep| req_deps.contains(&dep.name.as_str()))
        .map(get_cargo_dependency)
        .collect()
}

fn generate_builder_call(
    builder_ident: &syn::Ident,
    mod_path: &str,
    fn_name: &str,
) -> proc_macro2::TokenStream {
    let syn_func_path = syn::parse_str::<syn::Path>(&format!("{mod_path}::{fn_name}"))
        .expect("Failed to parse path.");

    quote! {
        #syn_func_path(&mut #builder_ident);
    }
}

fn generate_main_rs(
    user_crate_name: &str,
    main_backend: &BackendType,
    functions: &[FunctionMetadata],
) -> String {
    let backend_types = backend::generate_backend_typedef_stream(main_backend);
    let (_backend_type_name, _autodiff_backend_type_name) = backend::get_backend_type_names();
    let backend_default_device = main_backend.default_device_stream();

    let builder_ident = syn::Ident::new("builder", proc_macro2::Span::call_site());
    let builder_registration: Vec<proc_macro2::TokenStream> = functions
        .iter()
        .map(|flag| {
            let proc_call =
                generate_builder_call(&builder_ident, &flag.mod_path, &flag.builder_fn_name);
            quote! {
                #proc_call
            }
        })
        .collect();

    let recursion_limit = if matches!(main_backend, BackendType::Wgpu) {
        quote! {
            #![recursion_limit = "256"]
        }
    } else {
        quote! {}
    };

    let crate_name_str = syn::Ident::new(
        &user_crate_name.to_lowercase().replace('-', "_"),
        proc_macro2::Span::call_site(),
    );

    let bin_content: proc_macro2::TokenStream = quote! {
        #recursion_limit
        #backend_types

        use #crate_name_str::*;
        use burn::prelude::*;

        fn main() -> Result<(), String> {
            use burn_central::runtime::Executor;

            let runtime_args = burn_central::runtime::cli::parse_runtime_args();

            let device = #backend_default_device;

            let key = runtime_args.burn_central.api_key;
            let endpoint = runtime_args.burn_central.endpoint;
            let namespace = runtime_args.burn_central.namespace;
            let project = runtime_args.burn_central.project;

            let creds = burn_central::BurnCentralCredentials::new(key);
            let client = burn_central::BurnCentral::builder(creds)
                .with_endpoint(endpoint)
                .build()
                .map_err(|e| e.to_string())?;

            let project_path = burn_central::schemas::ProjectPath::try_from(format!("{}/{}", namespace, project))
                .expect("Project path should be valid");

            let mut #builder_ident = Executor::<MyAutodiffBackend>::builder();
            #(#builder_registration)*
            // #crate_entrypoint(&mut #builder_ident);

            #builder_ident
                .build(client, namespace, project)
                .run(
                    runtime_args.kind.parse().unwrap(),
                    runtime_args.routine,
                    [device],
                    Some(runtime_args.args),
                )
                .map_err(|e| e.to_string())
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
    functions: &[FunctionMetadata],
    current_pkg: &cargo_metadata::Package,
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
    find_required_dependencies(current_pkg, vec!["burn-central", "burn"])
        .drain(..)
        .for_each(|mut dep| {
            if dep.name == "burn" {
                burn_features.iter().for_each(|feature| {
                    dep.add_feature(feature.to_string());
                });
            }
            generated_crate.add_dependency(dep);
        });

    // Generate source files
    generated_crate.src_mut().insert(FileTree::new_file(
        "main.rs",
        generate_main_rs(user_project_name, backend, functions),
    ));

    generated_crate
}
