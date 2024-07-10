use quote::quote;
use std::path::PathBuf;

pub fn get_heat_dir() -> PathBuf {
    PathBuf::from(".heat")
}

fn generate_cargo_toml(project_name: &str, project_dir: &str, burn_features: Vec<&str>) -> String {
    let mut cargo_toml = toml_edit::DocumentMut::new();
    // package settings
    let mut package = toml_edit::Table::new();
    package.insert("edition", toml_edit::value("2021"));
    package.insert("version", toml_edit::value("0.1.0"));
    package.insert_formatted(
        &toml_edit::Key::new("name"),
        toml_edit::value("generated_heat_crate"),
    );

    // dependencies
    let mut dependencies = toml_edit::Table::new();
    dependencies[project_name]["path"] = toml_edit::value(project_dir);
    if let Some(t) = dependencies[project_name].as_inline_table_mut() {
        t.fmt()
    }

    dependencies["burn"]["git"] = toml_edit::value("https://github.com/tracel-ai/burn");
    dependencies["burn"]["branch"] = toml_edit::value("main");
    let mut burn_features_array = toml_edit::Array::new();
    burn_features_array.extend(burn_features);
    dependencies["burn"]["features"] =
        toml_edit::value(toml_edit::Value::Array(burn_features_array));

    dependencies["clap"]["version"] = toml_edit::value("*");
    if let Some(a) = dependencies["clap"]["features"].as_array_mut() {
        a.push("cargo");
    }

    // workspace
    let workspace = toml_edit::table();

    // insert into cargo_toml
    cargo_toml.insert("package", toml_edit::Item::Table(package));
    cargo_toml.insert("dependencies", toml_edit::Item::Table(dependencies));
    cargo_toml.insert("workspace", workspace);
    cargo_toml.to_string()
}

fn generate_main_rs() -> String {
    let flags = crate::registry::get_flags();

    let match_arms: Vec<_> = flags.iter().filter(|flag| flag.proc_type == "training").map(|flag| {
        let flag_name = flag.fn_name;
        let syn_func_path = syn::parse_str::<syn::Path>(&format!("{}::heat_training_main_{}",flag.mod_path, flag.fn_name)).expect("Failed to parse path.");

        quote! {
             #flag_name => {
                let _ = #syn_func_path(config_path.to_string(), key.to_string(), project.to_string(), heat_endpoint.to_string());
            }
        }
    }).collect();

    let train_func_match = quote! {
        match func.as_str() {
            #(#match_arms)*
            _ => panic!("Unknown training function: {}", func),
        }
    };

    let bin_content: proc_macro2::TokenStream = quote! {
        fn main() {
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

        let matches = command.get_matches();

        if let Some(train_matches) = matches.subcommand_matches("train") {
            let func = train_matches.get_one::<String>("func").expect("func should be set.");
            let config_path = train_matches.get_one::<String>("config").expect("config should be set.");
            let project = matches.get_one::<String>("project").expect("project should be set.");
            let key = matches.get_one::<String>("key").expect("key should be set.");
            let heat_endpoint = matches.get_one::<String>("heat-endpoint").expect("heat-endpoint should be set.");

            #train_func_match
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

pub fn create_crate(burn_features: Vec<&str>) {
    let project_dir =
        std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR should be set.");
    let project_name = std::env::var("CARGO_PKG_NAME").expect("CARGO_PKG_NAME should be set.");

    let mut crate_path = PathBuf::from(project_dir.clone());

    crate_path.push(get_heat_dir());
    std::fs::create_dir_all(&crate_path).expect("Should be able to create crate directory.");

    std::fs::write(crate_path.join(".gitignore"), "*")
        .expect("Should be able to write gitignore file.");

    crate_path.extend(["crates", "heat-sdk-cli"]);
    std::fs::create_dir_all(&crate_path).expect("Should be able to create crate directory.");

    // src + src/main.rs
    let mut main_path = crate_path.join("src");
    std::fs::create_dir_all(&main_path).expect("Should be able to create src directory.");
    main_path.push("main.rs");

    // generate and paste new code into main.rs if content has changed since last run
    let last_bin_content = std::fs::read_to_string(&main_path);
    let new_bin_content = generate_main_rs();

    let should_write = match last_bin_content {
        Ok(ref content) => content != &new_bin_content,
        Err(_) => true,
    };

    // todo hash comparison from merkle tree
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
