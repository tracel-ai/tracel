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
    dependencies[&project_name]["path"] = toml_edit::value(project_dir);
    dependencies[&project_name]
        .as_inline_table_mut()
        .map(|t| t.fmt());
    dependencies["burn"]["git"] = toml_edit::value("https://github.com/tracel-ai/burn");
    dependencies["burn"]["branch"] = toml_edit::value("main");

    let mut burn_features_array = toml_edit::Array::new();
    burn_features_array.extend(burn_features.into_iter());
    dependencies["burn"]["features"] =
        toml_edit::value(toml_edit::Value::Array(burn_features_array));

    // workspace
    let workspace = toml_edit::table();

    // insert into cargo_toml
    cargo_toml.insert("package", toml_edit::Item::Table(package));
    cargo_toml.insert("dependencies", toml_edit::Item::Table(dependencies));
    cargo_toml.insert("workspace", workspace);
    cargo_toml.to_string()
}

fn generate_main_rs() -> String {
    const BIN_CONTENT: &str = stringify!(
        fn main() {
            println!("Hello world!")
        }
    );

    // parse the bin content to check if it is valid and format it
    let syn_tree = syn::parse_file(BIN_CONTENT).expect("Failed to parse bin content");
    prettyplease::unparse(&syn_tree)
}

pub fn create_crate(burn_features: Vec<&str>) {
    let project_dir =
        std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR should be set.");
    let project_name = std::env::var("CARGO_PKG_NAME").expect("CARGO_PKG_NAME should be set.");

    let mut crate_path = PathBuf::from(project_dir.clone());
    crate_path.push(get_heat_dir());
    std::fs::write(crate_path.join(".gitignore"), "*\n./*\n.")
        .expect("Should be able to write gitignore file.");

    crate_path.extend(["crates", "heat-sdk-cli"]);
    std::fs::create_dir_all(&crate_path).expect("Should be able to create crate directory.");

    // src + src/main.rs
    let mut main_path = crate_path.join("src");
    std::fs::create_dir_all(&main_path).expect("Should be able to create src directory.");
    main_path.push("main.rs");

    let bin_content_formatted = generate_main_rs();

    std::fs::write(main_path, bin_content_formatted).expect("Failed to write bin file");

    // Cargo.toml
    let cargo_toml_str = generate_cargo_toml(&project_name, &project_dir, burn_features);

    let cargotoml_path = crate_path.join("Cargo.toml");
    std::fs::write(cargotoml_path, cargo_toml_str)
        .expect("Should be able to write Cargo.toml file.");
}
