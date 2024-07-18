pub struct Package {
    pub name: String,
    pub version: String,
    pub edition: String,
}

pub enum QueryType {
    Branch(String),
    Tag(String),
    Rev(String),
}

pub enum DependencyKind {
    Path(String),
    Git(String, QueryType),
    Registry(),
}

pub struct Dependency {
    pub name: String,
    pub version: String,
    pub kind: DependencyKind,
    pub features: Vec<String>,
}

impl Dependency {
    pub fn new(name: String, version: String, features: Vec<String>) -> Self {
        Self {
            name,
            version,
            kind: DependencyKind::Registry(),
            features,
        }
    }

    pub fn new_path(name: String, version: String, path: String, features: Vec<String>) -> Self {
        Self {
            name,
            version,
            kind: DependencyKind::Path(path),
            features,
        }
    }

    pub fn new_git(
        name: String,
        version: String,
        url: String,
        query: QueryType,
        features: Vec<String>,
    ) -> Self {
        Self {
            name,
            version,
            kind: DependencyKind::Git(url, query),
            features,
        }
    }

    pub fn add_feature(&mut self, feature: String) {
        self.features.push(feature);
    }

    pub fn remove_feature(&mut self, feature: &str) {
        self.features.retain(|f| f != feature);
    }

    pub fn set_version(&mut self, version: String) {
        self.version = version;
    }

    pub fn set_kind(&mut self, kind: DependencyKind) {
        self.kind = kind;
    }

    pub fn set_name(&mut self, name: String) {
        self.name = name;
    }
}

pub struct CargoToml {
    pub package: Package,
    pub dependencies: Vec<Dependency>,
}

impl CargoToml {
    pub fn new(package: Package, dependencies: Vec<Dependency>) -> Self {
        Self {
            package,
            dependencies,
        }
    }

    pub fn add_dependency(&mut self, dependency: Dependency) {
        self.dependencies.push(dependency);
    }

    pub fn remove_dependency(&mut self, name: &str) {
        self.dependencies.retain(|dep| dep.name != name);
    }

    pub fn set_package_name(&mut self, name: String) {
        self.package.name = name;
    }

    pub fn set_package_version(&mut self, version: String) {
        self.package.version = version;
    }

    pub fn set_package_edition(&mut self, edition: String) {
        self.package.edition = edition;
    }
}

impl Default for CargoToml {
    fn default() -> Self {
        Self {
            package: Package {
                name: "default".to_string(),
                version: "0.1.0".to_string(),
                edition: "2021".to_string(),
            },
            dependencies: vec![],
        }
    }
}

impl ToString for CargoToml {
    fn to_string(&self) -> String {
        let mut cargo_toml = toml_edit::DocumentMut::new();
        let mut package = toml_edit::table();
        package["edition"] = toml_edit::value(&self.package.edition);
        package["version"] = toml_edit::value(&self.package.version);
        package["name"] = toml_edit::value(&self.package.name);

        let mut dependencies = toml_edit::table();
        for dep in &self.dependencies {
            let mut dep_table = toml_edit::table();
            dep_table["version"] = toml_edit::value(&dep.version);
            if !dep.features.is_empty() {
                let mut feat_arr = toml_edit::Array::new();
                for feat in &dep.features {
                    feat_arr.push(feat);
                }
                dep_table["features"] = toml_edit::Item::Value(toml_edit::Value::Array(feat_arr));
            }
            match &dep.kind {
                DependencyKind::Path(path) => {
                    dep_table["path"] = toml_edit::value(path);
                }
                DependencyKind::Git(url, query) => {
                    match query {
                        QueryType::Branch(branch) => dep_table["branch"] = toml_edit::value(branch),
                        QueryType::Tag(tag) => dep_table["tag"] = toml_edit::value(tag),
                        QueryType::Rev(rev) => dep_table["rev"] = toml_edit::value(rev),
                    };
                    dep_table["git"] = toml_edit::value(url);
                }
                DependencyKind::Registry() => {}
            }
            dependencies[&dep.name] = dep_table;
        }

        cargo_toml["package"] = package;
        cargo_toml["dependencies"] = dependencies;

        cargo_toml["workspace"] = toml_edit::table();

        cargo_toml.to_string()
    }
}
