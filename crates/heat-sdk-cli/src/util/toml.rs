
use std::{collections::{BTreeMap, BTreeSet, HashSet}, ffi::OsStr, path::{Path, PathBuf}, str::FromStr};

use lazycell::LazyCell;

use cargo_metadata::{semver, PackageId};
use cargo_util_schemas::manifest::{self, RustVersion, StringOrBool, TomlManifest};

type CargoResult<T> = anyhow::Result<T>;

use anyhow::{bail, Context};

use crate::{paths, print_debug, print_info};

use super::{dependency::{DepKind, FeatureValue}, features::Edition, interning::InternedString, workspace::{resolve_relative_path, WorkspaceConfig, WorkspaceRootConfig}};

pub struct Manifest {
    pub original_toml: manifest::TomlManifest,
    pub resolved_toml: manifest::TomlManifest,
    pub workspace: WorkspaceConfig,
}

/// See also `bin/cargo/commands/run.rs`s `is_manifest_command`
pub fn is_embedded(path: &Path) -> bool {
    let ext = path.extension();
    ext == Some(OsStr::new("rs")) ||
        // Provide better errors by not considering directories to be embedded manifests
        (ext.is_none() && path.is_file())
}
fn read_toml_string(path: &Path) -> CargoResult<String> {
    let contents = paths::read(path)?;

    // won't consider for now
    
    // if is_embedded(path) {
    //     if !gctx.cli_unstable().script {
    //         anyhow::bail!("parsing `{}` requires `-Zscript`", path.display());
    //     }
    //     contents = embedded::expand_manifest(&contents, path)?;
    // }
    Ok(contents)
}

fn deserialize_toml(
    document: &toml_edit::ImDocument<String>,
) -> Result<manifest::TomlManifest, toml_edit::de::Error> {
    let mut unused = BTreeSet::new();
    let deserializer = toml_edit::de::Deserializer::from(document.clone());
    let mut document: manifest::TomlManifest = serde_ignored::deserialize(deserializer, |path| {
        let mut key = String::new();
        stringify(&mut key, &path);
        unused.insert(key);
    })?;
    document._unused_keys = unused;
    Ok(document)
}

fn stringify(dst: &mut String, path: &serde_ignored::Path<'_>) {
    use serde_ignored::Path;

    match *path {
        Path::Root => {}
        Path::Seq { parent, index } => {
            stringify(dst, parent);
            if !dst.is_empty() {
                dst.push('.');
            }
            dst.push_str(&index.to_string());
        }
        Path::Map { parent, ref key } => {
            stringify(dst, parent);
            if !dst.is_empty() {
                dst.push('.');
            }
            dst.push_str(key);
        }
        Path::Some { parent }
        | Path::NewtypeVariant { parent }
        | Path::NewtypeStruct { parent } => stringify(dst, parent),
    }
}

fn parse_document(contents: &str) -> Result<toml_edit::ImDocument<String>, toml_edit::de::Error> {
    toml_edit::ImDocument::parse(contents.to_owned()).map_err(Into::into)
}


/// Loads a `Cargo.toml` from a file on disk.
///
/// This could result in a real or virtual manifest being returned.
///
/// A list of nested paths is also returned, one for each path dependency
/// within the manifest. For virtual manifests, these paths can only
/// come from patched or replaced dependencies. These paths are not
/// canonicalized.
pub fn read_manifest(
    path: &Path,
    // source_id: PackageId,
    root_workspace_path: Option<&Path>
) -> CargoResult<Manifest> {
    let mut warnings = Default::default();
    let mut errors = Default::default();

    let contents = read_toml_string(path).with_context(|| format!("Manifest error: failed to read `{}`", path.display()))?;
    let document = parse_document(&contents).with_context(|| format!("Manifest error: failed to parse `{}`", path.display()))?;
    let original_toml = deserialize_toml(&document).with_context(|| format!("Manifest error: failed to deserialize `{}`", path.display()))?;

    // let empty = Vec::new();
    // let cargo_features = original_toml.cargo_features.as_ref().unwrap_or(&empty);
    let workspace_config = to_workspace_config(&original_toml, path)?;
    // if let WorkspaceConfig::Root(ws_root_config) = &workspace_config {
    //     let package_root = path.parent().unwrap();
    //     gctx.ws_roots
    //         .borrow_mut()
    //         .insert(package_root.to_owned(), ws_root_config.clone());
    // }
    let resolved_toml = resolve_toml(
        &original_toml,
        &workspace_config,
        path,
        root_workspace_path,
        &mut warnings,
        &mut errors,
    ).with_context(|| format!("failed to parse manifest at `{}`", path.display()))?;

    for warning in warnings {
        print_info!("{}", warning);
    }

    for error in errors {
        print_info!("{}", error);
    }

    Ok(Manifest {
        original_toml,
        resolved_toml,
        workspace: workspace_config,
    })
}

fn field_inherit_with<'a, T>(
    field: manifest::InheritableField<T>,
    label: &str,
    get_ws_inheritable: impl FnOnce() -> CargoResult<T>,
) -> CargoResult<T> {
    match field {
        manifest::InheritableField::Value(value) => Ok(value),
        manifest::InheritableField::Inherit(_) => get_ws_inheritable().with_context(|| {
            format!("`{}` was inherited but `{}` was not defined", label, label)
        }),
    }
}

const DEFAULT_README_FILES: [&str; 3] = ["README.md", "README.txt", "README"];

/// Checks if a file with any of the default README file names exists in the package root.
/// If so, returns a `String` representing that name.
fn default_readme_from_package_root(package_root: &Path) -> Option<String> {
    for &readme_filename in DEFAULT_README_FILES.iter() {
        if package_root.join(readme_filename).is_file() {
            return Some(readme_filename.to_string());
        }
    }

    None
}

/// Returns the name of the README file for a [`manifest::TomlPackage`].
fn resolve_package_readme(
    package_root: &Path,
    readme: Option<&manifest::StringOrBool>,
) -> Option<String> {
    match &readme {
        None => default_readme_from_package_root(package_root),
        Some(value) => match value {
            manifest::StringOrBool::Bool(false) => None,
            manifest::StringOrBool::Bool(true) => Some("README.md".to_string()),
            manifest::StringOrBool::String(v) => Some(v.clone()),
        },
    }
}

pub fn resolve_build(build: Option<&StringOrBool>, package_root: &Path) -> Option<StringOrBool> {
    const BUILD_RS: &str = "build.rs";
    match build {
        None => {
            // If there is a `build.rs` file next to the `Cargo.toml`, assume it is
            // a build script.
            let build_rs = package_root.join(BUILD_RS);
            if build_rs.is_file() {
                Some(StringOrBool::String(BUILD_RS.to_owned()))
            } else {
                Some(StringOrBool::Bool(false))
            }
        }
        // Explicitly no build script.
        Some(StringOrBool::Bool(false)) | Some(StringOrBool::String(_)) => build.cloned(),
        Some(StringOrBool::Bool(true)) => Some(StringOrBool::String(BUILD_RS.to_owned())),
    }
}


fn resolve_package_toml<'a>(
    original_package: &manifest::TomlPackage,
    package_root: &Path,
    inherit: &dyn Fn() -> CargoResult<&'a InheritableFields>,
) -> CargoResult<Box<manifest::TomlPackage>> {
    let resolved_package = manifest::TomlPackage {
        edition: original_package
            .edition
            .clone()
            .map(|value| field_inherit_with(value, "edition", || inherit()?.edition()))
            .transpose()?
            .map(manifest::InheritableField::Value),
        rust_version: original_package
            .rust_version
            .clone()
            .map(|value| field_inherit_with(value, "rust-version", || inherit()?.rust_version()))
            .transpose()?
            .map(manifest::InheritableField::Value),
        name: original_package.name.clone(),
        version: original_package
            .version
            .clone()
            .map(|value| field_inherit_with(value, "version", || inherit()?.version()))
            .transpose()?
            .map(manifest::InheritableField::Value),
        authors: original_package
            .authors
            .clone()
            .map(|value| field_inherit_with(value, "authors", || inherit()?.authors()))
            .transpose()?
            .map(manifest::InheritableField::Value),
        build: resolve_build(original_package.build.as_ref(), package_root),
        metabuild: original_package.metabuild.clone(),
        default_target: original_package.default_target.clone(),
        forced_target: original_package.forced_target.clone(),
        links: original_package.links.clone(),
        exclude: original_package
            .exclude
            .clone()
            .map(|value| field_inherit_with(value, "exclude", || inherit()?.exclude()))
            .transpose()?
            .map(manifest::InheritableField::Value),
        include: original_package
            .include
            .clone()
            .map(|value| field_inherit_with(value, "include", || inherit()?.include()))
            .transpose()?
            .map(manifest::InheritableField::Value),
        publish: original_package
            .publish
            .clone()
            .map(|value| field_inherit_with(value, "publish", || inherit()?.publish()))
            .transpose()?
            .map(manifest::InheritableField::Value),
        workspace: original_package.workspace.clone(),
        im_a_teapot: original_package.im_a_teapot.clone(),
        autobins: Some(false),
        autoexamples: Some(false),
        autotests: Some(false),
        autobenches: Some(false),
        default_run: original_package.default_run.clone(),
        description: original_package
            .description
            .clone()
            .map(|value| field_inherit_with(value, "description", || inherit()?.description()))
            .transpose()?
            .map(manifest::InheritableField::Value),
        homepage: original_package
            .homepage
            .clone()
            .map(|value| field_inherit_with(value, "homepage", || inherit()?.homepage()))
            .transpose()?
            .map(manifest::InheritableField::Value),
        documentation: original_package
            .documentation
            .clone()
            .map(|value| field_inherit_with(value, "documentation", || inherit()?.documentation()))
            .transpose()?
            .map(manifest::InheritableField::Value),
        readme: resolve_package_readme(
            package_root,
            original_package
                .readme
                .clone()
                .map(|value| {
                    field_inherit_with(value, "readme", || inherit()?.readme(package_root))
                })
                .transpose()?
                .as_ref(),
        )
        .map(|s| manifest::InheritableField::Value(StringOrBool::String(s)))
        .or(Some(manifest::InheritableField::Value(StringOrBool::Bool(
            false,
        )))),
        keywords: original_package
            .keywords
            .clone()
            .map(|value| field_inherit_with(value, "keywords", || inherit()?.keywords()))
            .transpose()?
            .map(manifest::InheritableField::Value),
        categories: original_package
            .categories
            .clone()
            .map(|value| field_inherit_with(value, "categories", || inherit()?.categories()))
            .transpose()?
            .map(manifest::InheritableField::Value),
        license: original_package
            .license
            .clone()
            .map(|value| field_inherit_with(value, "license", || inherit()?.license()))
            .transpose()?
            .map(manifest::InheritableField::Value),
        license_file: original_package
            .license_file
            .clone()
            .map(|value| {
                field_inherit_with(value, "license-file", || {
                    inherit()?.license_file(package_root)
                })
            })
            .transpose()?
            .map(manifest::InheritableField::Value),
        repository: original_package
            .repository
            .clone()
            .map(|value| field_inherit_with(value, "repository", || inherit()?.repository()))
            .transpose()?
            .map(manifest::InheritableField::Value),
        resolver: original_package.resolver.clone(),
        metadata: original_package.metadata.clone(),
        _invalid_cargo_features: Default::default(),
    };

    // if resolved_package.resolver.as_deref() == Some("3") {
    //     features.require(Feature::edition2024())?;
    // }

    Ok(Box::new(resolved_package))
}

/// See [`Manifest::resolved_toml`] for more details
fn resolve_toml(
    original_toml: &manifest::TomlManifest,
    workspace_config: &WorkspaceConfig,
    manifest_file: &Path,
    root_workspace_path: Option<&Path>,
    warnings: &mut Vec<String>,
    errors: &mut Vec<String>,
) -> CargoResult<manifest::TomlManifest> {
    // print_debug!("resolving toml: {:?}, {:?}, {:?}, {:?}", original_toml, workspace_config, manifest_file, root_workspace_path);

    let mut resolved_toml = manifest::TomlManifest {
        cargo_features: original_toml.cargo_features.clone(),
        package: None,
        project: None,
        profile: original_toml.profile.clone(),
        lib: None,
        bin: None,
        example: None,
        test: None,
        bench: None,
        dependencies: None,
        dev_dependencies: None,
        dev_dependencies2: None,
        build_dependencies: None,
        build_dependencies2: None,
        features: None,
        target: None,
        replace: original_toml.replace.clone(),
        patch: original_toml.patch.clone(),
        workspace: original_toml.workspace.clone(),
        badges: None,
        lints: None,
        _unused_keys: Default::default(),
    };

    let package_root = manifest_file.parent().unwrap();

    let inherit_cell: LazyCell<InheritableFields> = LazyCell::new();
    let inherit = || {
        inherit_cell
            .try_borrow_with(|| load_inheritable_fields(manifest_file, &workspace_config, root_workspace_path))
    };

    if let Some(original_package) = original_toml.package() {
        let package_name = &original_package.name;

        let resolved_package =
            resolve_package_toml(original_package, package_root, &inherit)?;
        let edition = resolved_package
            .resolved_edition()
            .expect("previously resolved")
            .map_or(Edition::default(), |e| {
                Edition::from_str(&e).unwrap_or_default()
            });
        resolved_toml.package = Some(resolved_package);

        resolved_toml.features = resolve_features(original_toml.features.as_ref())?;

        resolved_toml.lib = targets::resolve_lib(
            original_toml.lib.as_ref(),
            package_root,
            &original_package.name,
            edition,
            warnings,
        )?;
        resolved_toml.bin = Some(targets::resolve_bins(
            original_toml.bin.as_ref(),
            package_root,
            &original_package.name,
            edition,
            original_package.autobins,
            warnings,
            errors,
            resolved_toml.lib.is_some(),
        )?);

        resolved_toml.example = Some(targets::resolve_examples(
            original_toml.example.as_ref(),
            package_root,
            edition,
            original_package.autoexamples,
            warnings,
            errors,
        )?);
        resolved_toml.test = Some(targets::resolve_tests(
            original_toml.test.as_ref(),
            package_root,
            edition,
            original_package.autotests,
            warnings,
            errors,
        )?);
        resolved_toml.bench = Some(targets::resolve_benches(
            original_toml.bench.as_ref(),
            package_root,
            edition,
            original_package.autobenches,
            warnings,
            errors,
        )?);

        let activated_opt_deps = resolved_toml
            .features
            .as_ref()
            .map(|map| {
                map.values()
                    .flatten()
                    .filter_map(|f| match FeatureValue::new(InternedString::new(f)) {
                        FeatureValue::Dep { dep_name } => Some(dep_name.as_str()),
                        _ => None,
                    })
                    .collect::<HashSet<_>>()
            })
            .unwrap_or_default();

        resolved_toml.dependencies = resolve_dependencies(
            edition,
            original_toml.dependencies.as_ref(),
            &activated_opt_deps,
            None,
            &inherit,
            package_root,
            warnings,
        )?;
        deprecated_underscore(
            &original_toml.dev_dependencies2,
            &original_toml.dev_dependencies,
            "dev-dependencies",
            package_name,
            "package",
            edition,
            warnings,
        )?;
        resolved_toml.dev_dependencies = resolve_dependencies(
            edition,
            original_toml.dev_dependencies(),
            &activated_opt_deps,
            Some(DepKind::Development),
            &inherit,
            package_root,
            warnings,
        )?;
        deprecated_underscore(
            &original_toml.build_dependencies2,
            &original_toml.build_dependencies,
            "build-dependencies",
            package_name,
            "package",
            edition,
            warnings,
        )?;
        resolved_toml.build_dependencies = resolve_dependencies(
            edition,
            original_toml.build_dependencies(),
            &activated_opt_deps,
            Some(DepKind::Build),
            &inherit,
            package_root,
            warnings,
        )?;
        let mut resolved_target = BTreeMap::new();
        for (name, platform) in original_toml.target.iter().flatten() {
            let resolved_dependencies = resolve_dependencies(
                edition,
                platform.dependencies.as_ref(),
                &activated_opt_deps,
                None,
                &inherit,
                package_root,
                warnings,
            )?;
            deprecated_underscore(
                &platform.dev_dependencies2,
                &platform.dev_dependencies,
                "dev-dependencies",
                name,
                "platform target",
                edition,
                warnings,
            )?;
            let resolved_dev_dependencies = resolve_dependencies(
                edition,
                platform.dev_dependencies(),
                &activated_opt_deps,
                Some(DepKind::Development),
                &inherit,
                package_root,
                warnings,
            )?;
            deprecated_underscore(
                &platform.build_dependencies2,
                &platform.build_dependencies,
                "build-dependencies",
                name,
                "platform target",
                edition,
                warnings,
            )?;
            let resolved_build_dependencies = resolve_dependencies(
                edition,
                platform.build_dependencies(),
                &activated_opt_deps,
                Some(DepKind::Build),
                &inherit,
                package_root,
                warnings,
            )?;
            resolved_target.insert(
                name.clone(),
                manifest::TomlPlatform {
                    dependencies: resolved_dependencies,
                    build_dependencies: resolved_build_dependencies,
                    build_dependencies2: None,
                    dev_dependencies: resolved_dev_dependencies,
                    dev_dependencies2: None,
                },
            );
        }
        resolved_toml.target = (!resolved_target.is_empty()).then_some(resolved_target);

        let resolved_lints = original_toml
            .lints
            .clone()
            .map(|value| lints_inherit_with(value, || inherit()?.lints()))
            .transpose()?;
        resolved_toml.lints = resolved_lints.map(|lints| manifest::InheritableLints {
            workspace: false,
            lints,
        });

        resolved_toml.badges = original_toml.badges.clone();
    } else {
        for field in original_toml.requires_package() {
            bail!("this virtual manifest specifies a `{field}` section, which is not allowed");
        }
    }

    Ok(resolved_toml)
}

fn resolve_features(
    original_features: Option<&BTreeMap<manifest::FeatureName, Vec<String>>>,
) -> CargoResult<Option<BTreeMap<manifest::FeatureName, Vec<String>>>> {
    let Some(resolved_features) = original_features.cloned() else {
        return Ok(None);
    };

    Ok(Some(resolved_features))
}

fn resolve_dependencies<'a>(
    edition: Edition,
    orig_deps: Option<&BTreeMap<manifest::PackageName, manifest::InheritableDependency>>,
    activated_opt_deps: &HashSet<&str>,
    kind: Option<DepKind>,
    inherit: &dyn Fn() -> CargoResult<&'a InheritableFields>,
    package_root: &Path,
    warnings: &mut Vec<String>,
) -> CargoResult<Option<BTreeMap<manifest::PackageName, manifest::InheritableDependency>>> {
    let Some(dependencies) = orig_deps else {
        return Ok(None);
    };

    let mut deps = BTreeMap::new();
    for (name_in_toml, v) in dependencies.iter() {
        let mut resolved = dependency_inherit_with(
            v.clone(),
            name_in_toml,
            inherit,
            package_root,
            edition,
            warnings,
        )?;
        if let manifest::TomlDependency::Detailed(ref mut d) = resolved {
            deprecated_underscore(
                &d.default_features2,
                &d.default_features,
                "default-features",
                name_in_toml,
                "dependency",
                edition,
                warnings,
            )?;

            // Note here cargo would check for the `public` field, which is not stable at the time of writing
            // so we will ignore it for now
        }

        // if the dependency is not optional, it is always used
        // if the dependency is optional and activated, it is used
        // if the dependency is optional and not activated, it is not used
        let is_dep_activated =
            !resolved.is_optional() || activated_opt_deps.contains(name_in_toml.as_str());
        // If the edition is less than 2024, we don't need to check for unused optional dependencies
        if edition < Edition::Edition2024 || is_dep_activated {
            deps.insert(
                name_in_toml.clone(),
                manifest::InheritableDependency::Value(resolved.clone()),
            );
        }
    }
    Ok(Some(deps))
}

/// Warn about paths that have been deprecated and may conflict.
fn deprecated_underscore<T>(
    old: &Option<T>,
    new: &Option<T>,
    new_path: &str,
    name: &str,
    kind: &str,
    edition: Edition,
    warnings: &mut Vec<String>,
) -> CargoResult<()> {
    let old_path = new_path.replace("-", "_");
    if old.is_some() && Edition::Edition2024 <= edition {
        anyhow::bail!("`{old_path}` is unsupported as of the 2024 edition; instead use `{new_path}`\n(in the `{name}` {kind})");
    } else if old.is_some() && new.is_some() {
        warnings.push(format!(
            "`{old_path}` is redundant with `{new_path}`, preferring `{new_path}` in the `{name}` {kind}"
        ))
    } else if old.is_some() {
        warnings.push(format!(
            "`{old_path}` is deprecated in favor of `{new_path}` and will not work in the 2024 edition\n(in the `{name}` {kind})"
        ))
    }
    Ok(())
}

fn lints_inherit_with(
    lints: manifest::InheritableLints,
    get_ws_inheritable: impl FnOnce() -> CargoResult<manifest::TomlLints>,
) -> CargoResult<manifest::TomlLints> {
    if lints.workspace {
        if !lints.lints.is_empty() {
            anyhow::bail!("cannot override `workspace.lints` in `lints`, either remove the overrides or `lints.workspace = true` and manually specify the lints");
        }
        get_ws_inheritable().with_context(|| {
            "error inheriting `lints` from workspace root manifest's `workspace.lints`"
        })
    } else {
        Ok(lints.lints)
    }
}

fn dependency_inherit_with<'a>(
    dependency: manifest::InheritableDependency,
    name: &str,
    inherit: &dyn Fn() -> CargoResult<&'a InheritableFields>,
    package_root: &Path,
    edition: Edition,
    warnings: &mut Vec<String>,
) -> CargoResult<manifest::TomlDependency> {
    match dependency {
        manifest::InheritableDependency::Value(value) => Ok(value),
        manifest::InheritableDependency::Inherit(w) => {
            inner_dependency_inherit_with(w, name, inherit, package_root, edition, warnings).with_context(|| {
                format!(
                    "error inheriting `{name}` from workspace root manifest's `workspace.dependencies.{name}`",
                )
            })
        }
    }
}

fn inner_dependency_inherit_with<'a>(
    pkg_dep: manifest::TomlInheritedDependency,
    name: &str,
    inherit: &dyn Fn() -> CargoResult<&'a InheritableFields>,
    package_root: &Path,
    edition: Edition,
    warnings: &mut Vec<String>,
) -> CargoResult<manifest::TomlDependency> {
    let ws_dep = inherit()?.get_dependency(name, package_root)?;
    let mut merged_dep = match ws_dep {
        manifest::TomlDependency::Simple(ws_version) => manifest::TomlDetailedDependency {
            version: Some(ws_version),
            ..Default::default()
        },
        manifest::TomlDependency::Detailed(ws_dep) => ws_dep.clone(),
    };
    let manifest::TomlInheritedDependency {
        workspace: _,

        features,
        optional,
        default_features,
        default_features2,
        public,

        _unused_keys: _,
    } = &pkg_dep;
    let default_features = default_features.or(*default_features2);

    match (default_features, merged_dep.default_features()) {
        // member: default-features = true and
        // workspace: default-features = false should turn on
        // default-features
        (Some(true), Some(false)) => {
            merged_dep.default_features = Some(true);
        }
        // member: default-features = false and
        // workspace: default-features = true should ignore member
        // default-features
        (Some(false), Some(true)) => {
            deprecated_ws_default_features(name, Some(true), edition, warnings)?;
        }
        // member: default-features = false and
        // workspace: dep = "1.0" should ignore member default-features
        (Some(false), None) => {
            deprecated_ws_default_features(name, None, edition, warnings)?;
        }
        _ => {}
    }
    merged_dep.features = match (merged_dep.features.clone(), features.clone()) {
        (Some(dep_feat), Some(inherit_feat)) => Some(
            dep_feat
                .into_iter()
                .chain(inherit_feat)
                .collect::<Vec<String>>(),
        ),
        (Some(dep_fet), None) => Some(dep_fet),
        (None, Some(inherit_feat)) => Some(inherit_feat),
        (None, None) => None,
    };
    merged_dep.optional = *optional;
    merged_dep.public = *public;
    Ok(manifest::TomlDependency::Detailed(merged_dep))
}

fn deprecated_ws_default_features(
    label: &str,
    ws_def_feat: Option<bool>,
    edition: Edition,
    warnings: &mut Vec<String>,
) -> CargoResult<()> {
    let ws_def_feat = match ws_def_feat {
        Some(true) => "true",
        Some(false) => "false",
        None => "not specified",
    };
    if Edition::Edition2024 <= edition {
        anyhow::bail!("`default-features = false` cannot override workspace's `default-features`");
    } else {
        warnings.push(format!(
            "`default-features` is ignored for {label}, since `default-features` was \
                {ws_def_feat} for `workspace.dependencies.{label}`, \
                this could become a hard error in the future"
        ));
    }
    Ok(())
}


fn load_inheritable_fields(
    resolved_path: &Path,
    workspace_config: &WorkspaceConfig,
    root_workspace_path: Option<&Path>,
) -> CargoResult<InheritableFields> {
    match workspace_config {
        WorkspaceConfig::Root(root) => Ok(root.inheritable().clone()),
        WorkspaceConfig::Member {
            root: Some(ref path_to_root),
        } => {
            let path = resolved_path
                .parent()
                .unwrap()
                .join(path_to_root)
                .join("Cargo.toml");
            let root_path = crate::paths::normalize_path(&path);
            inheritable_from_path(root_path, root_workspace_path)
        }
        WorkspaceConfig::Member { root: None } => {
            match root_workspace_path {
                Some(path_to_root) => {
                    // let path = resolved_path
                    //     .parent()
                    //     .unwrap()
                    //     .join(path_to_root)
                    //     .join("Cargo.toml");
                    // let root_path = crate::paths::normalize_path(&path);
                    inheritable_from_path(path_to_root.to_path_buf(), root_workspace_path)
                }
                None => {
                    bail!("failed to find a workspace root");
                }
            }
        }
    }
}

fn inheritable_from_path(
    workspace_path: PathBuf,
    root_workspace_path: Option<&Path>
) -> CargoResult<InheritableFields> {
    // Workspace path should have Cargo.toml at the end
    let workspace_path_root = workspace_path.parent().unwrap();

    // Let the borrow exit scope so that it can be picked up if there is a need to
    // read a manifest
    // if let Some(ws_root) = gctx.ws_roots.borrow().get(workspace_path_root) {
        // return Ok(ws_root.inheritable().clone());
    // };

    if let Some(root_workspace_path) = root_workspace_path {
        if workspace_path_root == root_workspace_path {
            bail!("root of a workspace inferred but wasn't a root: {}", workspace_path.display());
        }
    }

    // let source_id = SourceId::for_path(workspace_path_root)?;
    let man = read_manifest(&workspace_path, root_workspace_path)?;
    match man.workspace {
        WorkspaceConfig::Root(root) => {
            // gctx.ws_roots
            //     .borrow_mut()
            //     .insert(workspace_path, root.clone());
            Ok(root.inheritable().clone())
        }
        _ => bail!(
            "root of a workspace inferred but wasn't a root: {}",
            workspace_path.display()
        ),
    }
}

fn to_workspace_config(
    original_toml: &manifest::TomlManifest,
    manifest_file: &Path,
) -> CargoResult<WorkspaceConfig> {
    let workspace_config = match (
        original_toml.workspace.as_ref(),
        original_toml.package().and_then(|p| p.workspace.as_ref()),
    ) {
        (Some(toml_config), None) => {
            // verify_lints(toml_config.lints.as_ref(), gctx, warnings)?;
            if let Some(ws_deps) = &toml_config.dependencies {
                for (name, dep) in ws_deps {
                    if dep.is_optional() {
                        bail!("{name} is optional, but workspace dependencies cannot be optional",);
                    }
                    if dep.is_public() {
                        bail!("{name} is public, but workspace dependencies cannot be public",);
                    }
                }

                for (name, dep) in ws_deps {
                    // unused_dep_keys(name, "workspace.dependencies", dep.unused_keys(), warnings);
                }
            }
            let ws_root_config = to_workspace_root_config(toml_config, manifest_file);
            WorkspaceConfig::Root(ws_root_config)
        }
        (None, root) => WorkspaceConfig::Member {
            root: root.cloned(),
        },
        (Some(..), Some(..)) => bail!(
            "cannot configure both `package.workspace` and \
                 `[workspace]`, only one can be specified"
        ),
    };
    Ok(workspace_config)
}

fn to_workspace_root_config(
    resolved_toml: &manifest::TomlWorkspace,
    manifest_file: &Path,
) -> WorkspaceRootConfig {
    let package_root = manifest_file.parent().unwrap();
    let inheritable = InheritableFields {
        package: resolved_toml.package.clone(),
        dependencies: resolved_toml.dependencies.clone(),
        lints: resolved_toml.lints.clone(),
        _ws_root: package_root.to_owned(),
    };
    let ws_root_config = WorkspaceRootConfig::new(
        package_root,
        &resolved_toml.members,
        &resolved_toml.default_members,
        &resolved_toml.exclude,
        &Some(inheritable),
        &resolved_toml.metadata,
    );
    ws_root_config
}

/// Defines simple getter methods for inheritable fields.
macro_rules! package_field_getter {
    ( $(($key:literal, $field:ident -> $ret:ty),)* ) => (
        $(
            #[doc = concat!("Gets the field `workspace.package", $key, "`.")]
            fn $field(&self) -> CargoResult<$ret> {
                let Some(val) = self.package.as_ref().and_then(|p| p.$field.as_ref()) else  {
                    bail!("`workspace.package.{}` was not defined", $key);
                };
                Ok(val.clone())
            }
        )*
    )
}

/// A group of fields that are inheritable by members of the workspace
#[derive(Clone, Debug, Default)]
pub struct InheritableFields {
    package: Option<manifest::InheritablePackage>,
    dependencies: Option<BTreeMap<manifest::PackageName, manifest::TomlDependency>>,
    lints: Option<manifest::TomlLints>,

    // Bookkeeping to help when resolving values from above
    _ws_root: PathBuf,
}

impl InheritableFields {
    package_field_getter! {
        // Please keep this list lexicographically ordered.
        ("authors",       authors       -> Vec<String>),
        ("categories",    categories    -> Vec<String>),
        ("description",   description   -> String),
        ("documentation", documentation -> String),
        ("edition",       edition       -> String),
        ("exclude",       exclude       -> Vec<String>),
        ("homepage",      homepage      -> String),
        ("include",       include       -> Vec<String>),
        ("keywords",      keywords      -> Vec<String>),
        ("license",       license       -> String),
        ("publish",       publish       -> manifest::VecStringOrBool),
        ("repository",    repository    -> String),
        ("rust-version",  rust_version  -> RustVersion),
        ("version",       version       -> semver::Version),
    }

    /// Gets a workspace dependency with the `name`.
    fn get_dependency(
        &self,
        name: &str,
        package_root: &Path,
    ) -> CargoResult<manifest::TomlDependency> {
        let Some(deps) = &self.dependencies else {
            bail!("`workspace.dependencies` was not defined");
        };
        let Some(dep) = deps.get(name) else {
            bail!("`dependency.{name}` was not found in `workspace.dependencies`");
        };
        let mut dep = dep.clone();
        if let manifest::TomlDependency::Detailed(detailed) = &mut dep {
            if let Some(rel_path) = &detailed.path {
                detailed.path = Some(resolve_relative_path(
                    name,
                    self.ws_root(),
                    package_root,
                    rel_path,
                )?);
            }
        }
        Ok(dep)
    }

    /// Gets the field `workspace.lint`.
    pub fn lints(&self) -> CargoResult<manifest::TomlLints> {
        let Some(val) = &self.lints else {
            bail!("`workspace.lints` was not defined");
        };
        Ok(val.clone())
    }

    /// Gets the field `workspace.package.license-file`.
    fn license_file(&self, package_root: &Path) -> CargoResult<String> {
        let Some(license_file) = self.package.as_ref().and_then(|p| p.license_file.as_ref()) else {
            bail!("`workspace.package.license-file` was not defined");
        };
        resolve_relative_path("license-file", &self._ws_root, package_root, license_file)
    }

    /// Gets the field `workspace.package.readme`.
    fn readme(&self, package_root: &Path) -> CargoResult<manifest::StringOrBool> {
        let Some(readme) = resolve_package_readme(
            self._ws_root.as_path(),
            self.package.as_ref().and_then(|p| p.readme.as_ref()),
        ) else {
            bail!("`workspace.package.readme` was not defined");
        };
        resolve_relative_path("readme", &self._ws_root, package_root, &readme)
            .map(manifest::StringOrBool::String)
    }
    
    fn ws_root(&self) -> &PathBuf {
        &self._ws_root
    }
}

mod targets {
//! This module implements Cargo conventions for directory layout:
//!
//!  * `src/lib.rs` is a library
//!  * `src/main.rs` is a binary
//!  * `src/bin/*.rs` are binaries
//!  * `examples/*.rs` are examples
//!  * `tests/*.rs` are integration tests
//!  * `benches/*.rs` are benchmarks
//!
//! It is a bit tricky because we need match explicit information from `Cargo.toml`
//! with implicit info in directory layout.

use std::collections::HashSet;
use std::fs::{self, DirEntry};
use std::path::{Path, PathBuf};

use cargo_util_schemas::manifest::{
    PathValue, StringOrBool, StringOrVec, TomlBenchTarget, TomlBinTarget, TomlExampleTarget,
    TomlLibTarget, TomlManifest, TomlTarget, TomlTestTarget,
};


use super::CargoResult;
use crate::print_debug;
use crate::util::features::Edition;
use crate::util::restricted_names;
use crate::util::toml::deprecated_underscore;

const DEFAULT_TEST_DIR_NAME: &'static str = "tests";
const DEFAULT_BENCH_DIR_NAME: &'static str = "benches";
const DEFAULT_EXAMPLE_DIR_NAME: &'static str = "examples";

// pub(super) fn to_targets(
//     features: &Features,
//     original_toml: &TomlManifest,
//     resolved_toml: &TomlManifest,
//     package_root: &Path,
//     edition: Edition,
//     metabuild: &Option<StringOrVec>,
//     warnings: &mut Vec<String>,
// ) -> CargoResult<Vec<Target>> {
//     let mut targets = Vec::new();

//     if let Some(target) = to_lib_target(
//         original_toml.lib.as_ref(),
//         resolved_toml.lib.as_ref(),
//         package_root,
//         edition,
//         warnings,
//     )? {
//         targets.push(target);
//     }

//     let package = resolved_toml
//         .package
//         .as_ref()
//         .ok_or_else(|| anyhow::format_err!("manifest has no `package` (or `project`)"))?;

//     targets.extend(to_bin_targets(
//         features,
//         resolved_toml.bin.as_deref().unwrap_or_default(),
//         package_root,
//         edition,
//     )?);

//     targets.extend(to_example_targets(
//         resolved_toml.example.as_deref().unwrap_or_default(),
//         package_root,
//         edition,
//     )?);

//     targets.extend(to_test_targets(
//         resolved_toml.test.as_deref().unwrap_or_default(),
//         package_root,
//         edition,
//     )?);

//     targets.extend(to_bench_targets(
//         resolved_toml.bench.as_deref().unwrap_or_default(),
//         package_root,
//         edition,
//     )?);

//     // processing the custom build script
//     if let Some(custom_build) = package.resolved_build().expect("should be resolved") {
//         if metabuild.is_some() {
//             anyhow::bail!("cannot specify both `metabuild` and `build`");
//         }
//         let custom_build = Path::new(custom_build);
//         let name = format!(
//             "build-script-{}",
//             custom_build
//                 .file_stem()
//                 .and_then(|s| s.to_str())
//                 .unwrap_or("")
//         );
//         targets.push(Target::custom_build_target(
//             &name,
//             package_root.join(custom_build),
//             edition,
//         ));
//     }
//     if let Some(metabuild) = metabuild {
//         // Verify names match available build deps.
//         let bdeps = resolved_toml.build_dependencies.as_ref();
//         for name in &metabuild.0 {
//             if !bdeps.map_or(false, |bd| bd.contains_key(name.as_str())) {
//                 anyhow::bail!(
//                     "metabuild package `{}` must be specified in `build-dependencies`",
//                     name
//                 );
//             }
//         }

//         targets.push(Target::metabuild_target(&format!(
//             "metabuild-{}",
//             package.name
//         )));
//     }

//     Ok(targets)
// }

pub fn resolve_lib(
    original_lib: Option<&TomlLibTarget>,
    package_root: &Path,
    package_name: &str,
    edition: Edition,
    warnings: &mut Vec<String>,
) -> CargoResult<Option<TomlLibTarget>> {
    let inferred = inferred_lib(package_root);
    let lib = original_lib.cloned().or_else(|| {
        inferred.as_ref().map(|lib| TomlTarget {
            path: Some(PathValue(lib.clone())),
            ..TomlTarget::new()
        })
    });
    let Some(mut lib) = lib else { return Ok(None) };
    lib.name
        .get_or_insert_with(|| package_name.replace("-", "_"));

    // Check early to improve error messages
    validate_lib_name(&lib, warnings)?;

    validate_proc_macro(&lib, "library", edition, warnings)?;
    validate_crate_types(&lib, "library", edition, warnings)?;

    if lib.path.is_none() {
        if let Some(inferred) = inferred {
            lib.path = Some(PathValue(inferred));
        } else {
            let name = name_or_panic(&lib);
            let legacy_path = Path::new("src").join(format!("{name}.rs"));
            if edition == Edition::Edition2015 && package_root.join(&legacy_path).exists() {
                warnings.push(format!(
                    "path `{}` was erroneously implicitly accepted for library `{name}`,\n\
                     please rename the file to `src/lib.rs` or set lib.path in Cargo.toml",
                    legacy_path.display(),
                ));
                lib.path = Some(PathValue(legacy_path));
            } else {
                anyhow::bail!(
                    "can't find library `{name}`, \
                     rename file to `src/lib.rs` or specify lib.path",
                )
            }
        }
    }

    Ok(Some(lib))
}

// fn to_lib_target(
//     original_lib: Option<&TomlLibTarget>,
//     resolved_lib: Option<&TomlLibTarget>,
//     package_root: &Path,
//     edition: Edition,
//     warnings: &mut Vec<String>,
// ) -> CargoResult<Option<Target>> {
//     let Some(lib) = resolved_lib else {
//         return Ok(None);
//     };

//     let path = lib.path.as_ref().expect("previously resolved");
//     let path = package_root.join(&path.0);

//     // Per the Macros 1.1 RFC:
//     //
//     // > Initially if a crate is compiled with the `proc-macro` crate type
//     // > (and possibly others) it will forbid exporting any items in the
//     // > crate other than those functions tagged #[proc_macro_derive] and
//     // > those functions must also be placed at the crate root.
//     //
//     // A plugin requires exporting plugin_registrar so a crate cannot be
//     // both at once.
//     let crate_types = match (lib.crate_types(), lib.proc_macro()) {
//         (Some(kinds), _)
//             if kinds.contains(&CrateType::Dylib.as_str().to_owned())
//                 && kinds.contains(&CrateType::Cdylib.as_str().to_owned()) =>
//         {
//             anyhow::bail!(format!(
//                 "library `{}` cannot set the crate type of both `dylib` and `cdylib`",
//                 name_or_panic(lib)
//             ));
//         }
//         (Some(kinds), _) if kinds.contains(&"proc-macro".to_string()) => {
//             warnings.push(format!(
//                 "library `{}` should only specify `proc-macro = true` instead of setting `crate-type`",
//                 name_or_panic(lib)
//             ));
//             if kinds.len() > 1 {
//                 anyhow::bail!("cannot mix `proc-macro` crate type with others");
//             }
//             vec![CrateType::ProcMacro]
//         }
//         (Some(kinds), _) => kinds.iter().map(|s| s.into()).collect(),
//         (None, Some(true)) => vec![CrateType::ProcMacro],
//         (None, _) => vec![CrateType::Lib],
//     };

//     let mut target = Target::lib_target(name_or_panic(lib), crate_types, path, edition);
//     configure(lib, &mut target)?;
//     target.set_name_inferred(original_lib.map_or(true, |v| v.name.is_none()));
//     Ok(Some(target))
// }

pub fn resolve_bins(
    toml_bins: Option<&Vec<TomlBinTarget>>,
    package_root: &Path,
    package_name: &str,
    edition: Edition,
    autodiscover: Option<bool>,
    warnings: &mut Vec<String>,
    errors: &mut Vec<String>,
    has_lib: bool,
) -> CargoResult<Vec<TomlBinTarget>> {
    if is_resolved(toml_bins, autodiscover) {
        let toml_bins = toml_bins.cloned().unwrap_or_default();
        for bin in &toml_bins {
            validate_bin_name(bin, warnings)?;
            validate_bin_crate_types(bin, edition, warnings, errors)?;
            validate_bin_proc_macro(bin, edition, warnings, errors)?;
        }
        Ok(toml_bins)
    } else {
        let inferred = inferred_bins(package_root, package_name);

        let mut bins = toml_targets_and_inferred(
            toml_bins,
            &inferred,
            package_root,
            autodiscover,
            edition,
            warnings,
            "binary",
            "bin",
            "autobins",
        );

        for bin in &mut bins {
            // Check early to improve error messages
            validate_bin_name(bin, warnings)?;

            validate_bin_crate_types(bin, edition, warnings, errors)?;
            validate_bin_proc_macro(bin, edition, warnings, errors)?;

            let path = target_path(bin, &inferred, "bin", package_root, edition, &mut |_| {
                if let Some(legacy_path) =
                    legacy_bin_path(package_root, name_or_panic(bin), has_lib)
                {
                    warnings.push(format!(
                        "path `{}` was erroneously implicitly accepted for binary `{}`,\n\
                     please set bin.path in Cargo.toml",
                        legacy_path.display(),
                        name_or_panic(bin)
                    ));
                    Some(legacy_path)
                } else {
                    None
                }
            });
            let path = match path {
                Ok(path) => path,
                Err(e) => anyhow::bail!("{}", e),
            };
            bin.path = Some(PathValue(path));
        }

        Ok(bins)
    }
}

// #[tracing::instrument(skip_all)]
// fn to_bin_targets(
//     features: &Features,
//     bins: &[TomlBinTarget],
//     package_root: &Path,
//     edition: Edition,
// ) -> CargoResult<Vec<Target>> {
//     // This loop performs basic checks on each of the TomlTarget in `bins`.
//     for bin in bins {
//         // For each binary, check if the `filename` parameter is populated. If it is,
//         // check if the corresponding cargo feature has been activated.
//         if bin.filename.is_some() {
//             features.require(Feature::different_binary_name())?;
//         }
//     }

//     validate_unique_names(&bins, "binary")?;

//     let mut result = Vec::new();
//     for bin in bins {
//         let path = package_root.join(&bin.path.as_ref().expect("previously resolved").0);
//         let mut target = Target::bin_target(
//             name_or_panic(bin),
//             bin.filename.clone(),
//             path,
//             bin.required_features.clone(),
//             edition,
//         );

//         configure(bin, &mut target)?;
//         result.push(target);
//     }
//     Ok(result)
// }

fn legacy_bin_path(package_root: &Path, name: &str, has_lib: bool) -> Option<PathBuf> {
    if !has_lib {
        let rel_path = Path::new("src").join(format!("{}.rs", name));
        if package_root.join(&rel_path).exists() {
            return Some(rel_path);
        }
    }

    let rel_path = Path::new("src").join("main.rs");
    if package_root.join(&rel_path).exists() {
        return Some(rel_path);
    }

    let default_bin_dir_name = Path::new("src").join("bin");
    let rel_path = default_bin_dir_name.join("main.rs");
    if package_root.join(&rel_path).exists() {
        return Some(rel_path);
    }
    None
}

pub fn resolve_examples(
    toml_examples: Option<&Vec<TomlExampleTarget>>,
    package_root: &Path,
    edition: Edition,
    autodiscover: Option<bool>,
    warnings: &mut Vec<String>,
    errors: &mut Vec<String>,
) -> CargoResult<Vec<TomlExampleTarget>> {
    let mut inferred = || infer_from_directory(&package_root, Path::new(DEFAULT_EXAMPLE_DIR_NAME));

    let targets = resolve_targets(
        "example",
        "example",
        toml_examples,
        &mut inferred,
        package_root,
        edition,
        autodiscover,
        warnings,
        errors,
        "autoexamples",
    )?;

    Ok(targets)
}

// fn to_example_targets(
//     targets: &[TomlExampleTarget],
//     package_root: &Path,
//     edition: Edition,
// ) -> CargoResult<Vec<Target>> {
//     validate_unique_names(&targets, "example")?;

//     let mut result = Vec::new();
//     for toml in targets {
//         let path = package_root.join(&toml.path.as_ref().expect("previously resolved").0);
//         let crate_types = match toml.crate_types() {
//             Some(kinds) => kinds.iter().map(|s| s.into()).collect(),
//             None => Vec::new(),
//         };

//         let mut target = Target::example_target(
//             name_or_panic(&toml),
//             crate_types,
//             path,
//             toml.required_features.clone(),
//             edition,
//         );
//         configure(&toml, &mut target)?;
//         result.push(target);
//     }

//     Ok(result)
// }

pub fn resolve_tests(
    toml_tests: Option<&Vec<TomlTestTarget>>,
    package_root: &Path,
    edition: Edition,
    autodiscover: Option<bool>,
    warnings: &mut Vec<String>,
    errors: &mut Vec<String>,
) -> CargoResult<Vec<TomlTestTarget>> {
    let mut inferred = || infer_from_directory(&package_root, Path::new(DEFAULT_TEST_DIR_NAME));

    let targets = resolve_targets(
        "test",
        "test",
        toml_tests,
        &mut inferred,
        package_root,
        edition,
        autodiscover,
        warnings,
        errors,
        "autotests",
    )?;

    Ok(targets)
}

// fn to_test_targets(
//     targets: &[TomlTestTarget],
//     package_root: &Path,
//     edition: Edition,
// ) -> CargoResult<Vec<Target>> {
//     validate_unique_names(&targets, "test")?;

//     let mut result = Vec::new();
//     for toml in targets {
//         let path = package_root.join(&toml.path.as_ref().expect("previously resolved").0);
//         let mut target = Target::test_target(
//             name_or_panic(&toml),
//             path,
//             toml.required_features.clone(),
//             edition,
//         );
//         configure(&toml, &mut target)?;
//         result.push(target);
//     }
//     Ok(result)
// }

pub fn resolve_benches(
    toml_benches: Option<&Vec<TomlBenchTarget>>,
    package_root: &Path,
    edition: Edition,
    autodiscover: Option<bool>,
    warnings: &mut Vec<String>,
    errors: &mut Vec<String>,
) -> CargoResult<Vec<TomlBenchTarget>> {
    let mut legacy_warnings = vec![];
    let mut legacy_bench_path = |bench: &TomlTarget| {
        let legacy_path = Path::new("src").join("bench.rs");
        if !(name_or_panic(bench) == "bench" && package_root.join(&legacy_path).exists()) {
            return None;
        }
        legacy_warnings.push(format!(
            "path `{}` was erroneously implicitly accepted for benchmark `{}`,\n\
                 please set bench.path in Cargo.toml",
            legacy_path.display(),
            name_or_panic(bench)
        ));
        Some(legacy_path)
    };

    let mut inferred = || infer_from_directory(&package_root, Path::new(DEFAULT_BENCH_DIR_NAME));

    let targets = resolve_targets_with_legacy_path(
        "benchmark",
        "bench",
        toml_benches,
        &mut inferred,
        package_root,
        edition,
        autodiscover,
        warnings,
        errors,
        &mut legacy_bench_path,
        "autobenches",
    )?;
    warnings.append(&mut legacy_warnings);

    Ok(targets)
}

// #[tracing::instrument(skip_all)]
// fn to_bench_targets(
//     targets: &[TomlBenchTarget],
//     package_root: &Path,
//     edition: Edition,
// ) -> CargoResult<Vec<Target>> {
//     validate_unique_names(&targets, "bench")?;

//     let mut result = Vec::new();
//     for toml in targets {
//         let path = package_root.join(&toml.path.as_ref().expect("previously resolved").0);
//         let mut target = Target::bench_target(
//             name_or_panic(&toml),
//             path,
//             toml.required_features.clone(),
//             edition,
//         );
//         configure(&toml, &mut target)?;
//         result.push(target);
//     }

//     Ok(result)
// }

fn is_resolved(toml_targets: Option<&Vec<TomlTarget>>, autodiscover: Option<bool>) -> bool {
    if autodiscover != Some(false) {
        return false;
    }

    let Some(toml_targets) = toml_targets else {
        return true;
    };
    toml_targets
        .iter()
        .all(|t| t.name.is_some() && t.path.is_some())
}

fn resolve_targets(
    target_kind_human: &str,
    target_kind: &str,
    toml_targets: Option<&Vec<TomlTarget>>,
    inferred: &mut dyn FnMut() -> Vec<(String, PathBuf)>,
    package_root: &Path,
    edition: Edition,
    autodiscover: Option<bool>,
    warnings: &mut Vec<String>,
    errors: &mut Vec<String>,
    autodiscover_flag_name: &str,
) -> CargoResult<Vec<TomlTarget>> {
    resolve_targets_with_legacy_path(
        target_kind_human,
        target_kind,
        toml_targets,
        inferred,
        package_root,
        edition,
        autodiscover,
        warnings,
        errors,
        &mut |_| None,
        autodiscover_flag_name,
    )
}

fn resolve_targets_with_legacy_path(
    target_kind_human: &str,
    target_kind: &str,
    toml_targets: Option<&Vec<TomlTarget>>,
    inferred: &mut dyn FnMut() -> Vec<(String, PathBuf)>,
    package_root: &Path,
    edition: Edition,
    autodiscover: Option<bool>,
    warnings: &mut Vec<String>,
    errors: &mut Vec<String>,
    legacy_path: &mut dyn FnMut(&TomlTarget) -> Option<PathBuf>,
    autodiscover_flag_name: &str,
) -> CargoResult<Vec<TomlTarget>> {
    if is_resolved(toml_targets, autodiscover) {
        let toml_targets = toml_targets.cloned().unwrap_or_default();
        for target in &toml_targets {
            // Check early to improve error messages
            validate_target_name(target, target_kind_human, target_kind, warnings)?;

            validate_proc_macro(target, target_kind_human, edition, warnings)?;
            validate_crate_types(target, target_kind_human, edition, warnings)?;
        }
        Ok(toml_targets)
    } else {
        let inferred = inferred();
        let toml_targets = toml_targets_and_inferred(
            toml_targets,
            &inferred,
            package_root,
            autodiscover,
            edition,
            warnings,
            target_kind_human,
            target_kind,
            autodiscover_flag_name,
        );

        for target in &toml_targets {
            // Check early to improve error messages
            validate_target_name(target, target_kind_human, target_kind, warnings)?;

            validate_proc_macro(target, target_kind_human, edition, warnings)?;
            validate_crate_types(target, target_kind_human, edition, warnings)?;
        }

        let mut result = Vec::new();
        for mut target in toml_targets {
            let path = target_path(
                &target,
                &inferred,
                target_kind,
                package_root,
                edition,
                legacy_path,
            );
            let path = match path {
                Ok(path) => path,
                Err(e) => {
                    errors.push(e);
                    continue;
                }
            };
            target.path = Some(PathValue(path));
            result.push(target);
        }
        Ok(result)
    }
}

fn inferred_lib(package_root: &Path) -> Option<PathBuf> {
    let lib = Path::new("src").join("lib.rs");
    if package_root.join(&lib).exists() {
        Some(lib)
    } else {
        None
    }
}

fn inferred_bins(package_root: &Path, package_name: &str) -> Vec<(String, PathBuf)> {
    let main = "src/main.rs";
    let mut result = Vec::new();
    if package_root.join(main).exists() {
        let main = PathBuf::from(main);
        result.push((package_name.to_string(), main));
    }
    let default_bin_dir_name = Path::new("src").join("bin");
    result.extend(infer_from_directory(package_root, &default_bin_dir_name));

    result
}

fn infer_from_directory(package_root: &Path, relpath: &Path) -> Vec<(String, PathBuf)> {
    let directory = package_root.join(relpath);
    let entries = match fs::read_dir(directory) {
        Err(_) => return Vec::new(),
        Ok(dir) => dir,
    };

    entries
        .filter_map(|e| e.ok())
        .filter(is_not_dotfile)
        .filter_map(|d| infer_any(package_root, &d))
        .collect()
}

fn infer_any(package_root: &Path, entry: &DirEntry) -> Option<(String, PathBuf)> {
    if entry.file_type().map_or(false, |t| t.is_dir()) {
        infer_subdirectory(package_root, entry)
    } else if entry.path().extension().and_then(|p| p.to_str()) == Some("rs") {
        infer_file(package_root, entry)
    } else {
        None
    }
}

fn infer_file(package_root: &Path, entry: &DirEntry) -> Option<(String, PathBuf)> {
    let path = entry.path();
    let stem = path.file_stem()?.to_str()?.to_owned();
    let path = path
        .strip_prefix(package_root)
        .map(|p| p.to_owned())
        .unwrap_or(path);
    Some((stem, path))
}

fn infer_subdirectory(package_root: &Path, entry: &DirEntry) -> Option<(String, PathBuf)> {
    let path = entry.path();
    let main = path.join("main.rs");
    let name = path.file_name()?.to_str()?.to_owned();
    if main.exists() {
        let main = main
            .strip_prefix(package_root)
            .map(|p| p.to_owned())
            .unwrap_or(main);
        Some((name, main))
    } else {
        None
    }
}

fn is_not_dotfile(entry: &DirEntry) -> bool {
    entry.file_name().to_str().map(|s| s.starts_with('.')) == Some(false)
}

fn toml_targets_and_inferred(
    toml_targets: Option<&Vec<TomlTarget>>,
    inferred: &[(String, PathBuf)],
    package_root: &Path,
    autodiscover: Option<bool>,
    edition: Edition,
    warnings: &mut Vec<String>,
    target_kind_human: &str,
    target_kind: &str,
    autodiscover_flag_name: &str,
) -> Vec<TomlTarget> {
    let inferred_targets = inferred_to_toml_targets(inferred);
    let mut toml_targets = match toml_targets {
        None => {
            if let Some(false) = autodiscover {
                vec![]
            } else {
                inferred_targets
            }
        }
        Some(targets) => {
            let mut targets = targets.clone();

            let target_path =
                |target: &TomlTarget| target.path.clone().map(|p| package_root.join(p.0));

            let mut seen_names = HashSet::new();
            let mut seen_paths = HashSet::new();
            for target in targets.iter() {
                seen_names.insert(target.name.clone());
                seen_paths.insert(target_path(target));
            }

            let mut rem_targets = vec![];
            for target in inferred_targets {
                if !seen_names.contains(&target.name) && !seen_paths.contains(&target_path(&target))
                {
                    rem_targets.push(target);
                }
            }

            let autodiscover = match autodiscover {
                Some(autodiscover) => autodiscover,
                None => {
                    if edition == Edition::Edition2015 {
                        if !rem_targets.is_empty() {
                            let mut rem_targets_str = String::new();
                            for t in rem_targets.iter() {
                                if let Some(p) = t.path.clone() {
                                    rem_targets_str.push_str(&format!("* {}\n", p.0.display()))
                                }
                            }
                            warnings.push(format!(
                                "\
An explicit [[{section}]] section is specified in Cargo.toml which currently
disables Cargo from automatically inferring other {target_kind_human} targets.
This inference behavior will change in the Rust 2018 edition and the following
files will be included as a {target_kind_human} target:

{rem_targets_str}
This is likely to break cargo build or cargo test as these files may not be
ready to be compiled as a {target_kind_human} target today. You can future-proof yourself
and disable this warning by adding `{autodiscover_flag_name} = false` to your [package]
section. You may also move the files to a location where Cargo would not
automatically infer them to be a target, such as in subfolders.

For more information on this warning you can consult
https://github.com/rust-lang/cargo/issues/5330",
                                section = target_kind,
                                target_kind_human = target_kind_human,
                                rem_targets_str = rem_targets_str,
                                autodiscover_flag_name = autodiscover_flag_name,
                            ));
                        };
                        false
                    } else {
                        true
                    }
                }
            };

            if autodiscover {
                targets.append(&mut rem_targets);
            }

            targets
        }
    };
    // Ensure target order is deterministic, particularly for `cargo vendor` where re-vendoring
    // should not cause changes.
    //
    // `unstable` should be deterministic because we enforce that `t.name` is unique
    toml_targets.sort_unstable_by_key(|t| t.name.clone());
    toml_targets
}

fn inferred_to_toml_targets(inferred: &[(String, PathBuf)]) -> Vec<TomlTarget> {
    inferred
        .iter()
        .map(|(name, path)| TomlTarget {
            name: Some(name.clone()),
            path: Some(PathValue(path.clone())),
            ..TomlTarget::new()
        })
        .collect()
}

/// Will check a list of toml targets, and make sure the target names are unique within a vector.
fn validate_unique_names(targets: &[TomlTarget], target_kind: &str) -> CargoResult<()> {
    let mut seen = HashSet::new();
    for name in targets.iter().map(|e| name_or_panic(e)) {
        if !seen.insert(name) {
            anyhow::bail!(
                "found duplicate {target_kind} name {name}, \
                 but all {target_kind} targets must have a unique name",
                target_kind = target_kind,
                name = name
            );
        }
    }
    Ok(())
}

// fn configure(toml: &TomlTarget, target: &mut Target) -> CargoResult<()> {
//     let t2 = target.clone();
//     target
//         .set_tested(toml.test.unwrap_or_else(|| t2.tested()))
//         .set_doc(toml.doc.unwrap_or_else(|| t2.documented()))
//         .set_doctest(toml.doctest.unwrap_or_else(|| t2.doctested()))
//         .set_benched(toml.bench.unwrap_or_else(|| t2.benched()))
//         .set_harness(toml.harness.unwrap_or_else(|| t2.harness()))
//         .set_proc_macro(toml.proc_macro().unwrap_or_else(|| t2.proc_macro()))
//         .set_doc_scrape_examples(match toml.doc_scrape_examples {
//             None => RustdocScrapeExamples::Unset,
//             Some(false) => RustdocScrapeExamples::Disabled,
//             Some(true) => RustdocScrapeExamples::Enabled,
//         })
//         .set_for_host(toml.proc_macro().unwrap_or_else(|| t2.for_host()));

//     if let Some(edition) = toml.edition.clone() {
//         target.set_edition(
//             edition
//                 .parse()
//                 .with_context(|| "failed to parse the `edition` key")?,
//         );
//     }
//     Ok(())
// }

/// Build an error message for a target path that cannot be determined either
/// by auto-discovery or specifying.
///
/// This function tries to detect commonly wrong paths for targets:
///
/// test -> tests/*.rs, tests/*/main.rs
/// bench -> benches/*.rs, benches/*/main.rs
/// example -> examples/*.rs, examples/*/main.rs
/// bin -> src/bin/*.rs, src/bin/*/main.rs
///
/// Note that the logic need to sync with [`infer_from_directory`] if changes.
fn target_path_not_found_error_message(
    package_root: &Path,
    target: &TomlTarget,
    target_kind: &str,
) -> String {
    fn possible_target_paths(name: &str, kind: &str, commonly_wrong: bool) -> [PathBuf; 2] {
        let mut target_path = PathBuf::new();
        match (kind, commonly_wrong) {
            // commonly wrong paths
            ("test" | "bench" | "example", true) => target_path.push(kind),
            ("bin", true) => {
                target_path.push("src");
                target_path.push("bins");
            }
            // default inferred paths
            ("test", false) => target_path.push(DEFAULT_TEST_DIR_NAME),
            ("bench", false) => target_path.push(DEFAULT_BENCH_DIR_NAME),
            ("example", false) => target_path.push(DEFAULT_EXAMPLE_DIR_NAME),
            ("bin", false) => {
                target_path.push("src");
                target_path.push("bin");
            }
            _ => unreachable!("invalid target kind: {}", kind),
        }
        target_path.push(name);

        let target_path_file = {
            let mut path = target_path.clone();
            path.set_extension("rs");
            path
        };
        let target_path_subdir = {
            target_path.push("main.rs");
            target_path
        };
        return [target_path_file, target_path_subdir];
    }

    let target_name = name_or_panic(target);
    let commonly_wrong_paths = possible_target_paths(&target_name, target_kind, true);
    let possible_paths = possible_target_paths(&target_name, target_kind, false);
    let existing_wrong_path_index = match (
        package_root.join(&commonly_wrong_paths[0]).exists(),
        package_root.join(&commonly_wrong_paths[1]).exists(),
    ) {
        (true, _) => Some(0),
        (_, true) => Some(1),
        _ => None,
    };

    if let Some(i) = existing_wrong_path_index {
        return format!(
            "\
can't find `{name}` {kind} at default paths, but found a file at `{wrong_path}`.
Perhaps rename the file to `{possible_path}` for target auto-discovery, \
or specify {kind}.path if you want to use a non-default path.",
            name = target_name,
            kind = target_kind,
            wrong_path = commonly_wrong_paths[i].display(),
            possible_path = possible_paths[i].display(),
        );
    }

    format!(
        "can't find `{name}` {kind} at `{path_file}` or `{path_dir}`. \
        Please specify {kind}.path if you want to use a non-default path.",
        name = target_name,
        kind = target_kind,
        path_file = possible_paths[0].display(),
        path_dir = possible_paths[1].display(),
    )
}

fn target_path(
    target: &TomlTarget,
    inferred: &[(String, PathBuf)],
    target_kind: &str,
    package_root: &Path,
    edition: Edition,
    legacy_path: &mut dyn FnMut(&TomlTarget) -> Option<PathBuf>,
) -> Result<PathBuf, String> {
    if let Some(ref path) = target.path {
        // Should we verify that this path exists here?
        return Ok(path.0.clone());
    }
    let name = name_or_panic(target).to_owned();

    let mut matching = inferred
        .iter()
        .filter(|(n, _)| n == &name)
        .map(|(_, p)| p.clone());

    let first = matching.next();
    let second = matching.next();
    match (first, second) {
        (Some(path), None) => Ok(path),
        (None, None) => {
            if edition == Edition::Edition2015 {
                if let Some(path) = legacy_path(target) {
                    return Ok(path);
                }
            }
            Err(target_path_not_found_error_message(
                package_root,
                target,
                target_kind,
            ))
        }
        (Some(p0), Some(p1)) => {
            if edition == Edition::Edition2015 {
                if let Some(path) = legacy_path(target) {
                    return Ok(path);
                }
            }
            Err(format!(
                "\
cannot infer path for `{}` {}
Cargo doesn't know which to use because multiple target files found at `{}` and `{}`.",
                name_or_panic(target),
                target_kind,
                p0.strip_prefix(package_root).unwrap_or(&p0).display(),
                p1.strip_prefix(package_root).unwrap_or(&p1).display(),
            ))
        }
        (None, Some(_)) => unreachable!(),
    }
}

/// Returns the path to the build script if one exists for this crate.
pub fn resolve_build(build: Option<&StringOrBool>, package_root: &Path) -> Option<StringOrBool> {
    const BUILD_RS: &str = "build.rs";
    match build {
        None => {
            // If there is a `build.rs` file next to the `Cargo.toml`, assume it is
            // a build script.
            let build_rs = package_root.join(BUILD_RS);
            if build_rs.is_file() {
                Some(StringOrBool::String(BUILD_RS.to_owned()))
            } else {
                Some(StringOrBool::Bool(false))
            }
        }
        // Explicitly no build script.
        Some(StringOrBool::Bool(false)) | Some(StringOrBool::String(_)) => build.cloned(),
        Some(StringOrBool::Bool(true)) => Some(StringOrBool::String(BUILD_RS.to_owned())),
    }
}

fn name_or_panic(target: &TomlTarget) -> &str {
    target
        .name
        .as_deref()
        .unwrap_or_else(|| panic!("target name is required"))
}

fn validate_lib_name(target: &TomlTarget, warnings: &mut Vec<String>) -> CargoResult<()> {
    validate_target_name(target, "library", "lib", warnings)?;
    let name = name_or_panic(target);
    if name.contains('-') {
        anyhow::bail!("library target names cannot contain hyphens: {}", name)
    }

    Ok(())
}

fn validate_bin_name(bin: &TomlTarget, warnings: &mut Vec<String>) -> CargoResult<()> {
    validate_target_name(bin, "binary", "bin", warnings)?;
    let name = name_or_panic(bin).to_owned();
    if restricted_names::is_conflicting_artifact_name(&name) {
        anyhow::bail!(
            "the binary target name `{name}` is forbidden, \
                 it conflicts with cargo's build directory names",
        )
    }

    Ok(())
}

fn validate_target_name(
    target: &TomlTarget,
    target_kind_human: &str,
    target_kind: &str,
    warnings: &mut Vec<String>,
) -> CargoResult<()> {
    match target.name {
        Some(ref name) => {
            if name.trim().is_empty() {
                anyhow::bail!("{} target names cannot be empty", target_kind_human)
            }
            if cfg!(windows) && restricted_names::is_windows_reserved(name) {
                warnings.push(format!(
                    "{} target `{}` is a reserved Windows filename, \
                        this target will not work on Windows platforms",
                    target_kind_human, name
                ));
            }
        }
        None => anyhow::bail!(
            "{} target {}.name is required",
            target_kind_human,
            target_kind
        ),
    }

    Ok(())
}

fn validate_bin_proc_macro(
    target: &TomlTarget,
    edition: Edition,
    warnings: &mut Vec<String>,
    errors: &mut Vec<String>,
) -> CargoResult<()> {
    if target.proc_macro() == Some(true) {
        let name = name_or_panic(target);
        errors.push(format!(
            "the target `{}` is a binary and can't have `proc-macro` \
                 set `true`",
            name
        ));
    } else {
        validate_proc_macro(target, "binary", edition, warnings)?;
    }
    Ok(())
}

fn validate_proc_macro(
    target: &TomlTarget,
    kind: &str,
    edition: Edition,
    warnings: &mut Vec<String>,
) -> CargoResult<()> {
    deprecated_underscore(
        &target.proc_macro2,
        &target.proc_macro,
        "proc-macro",
        name_or_panic(target),
        format!("{kind} target").as_str(),
        edition,
        warnings,
    )
}

fn validate_bin_crate_types(
    target: &TomlTarget,
    edition: Edition,
    warnings: &mut Vec<String>,
    errors: &mut Vec<String>,
) -> CargoResult<()> {
    if let Some(crate_types) = target.crate_types() {
        if !crate_types.is_empty() {
            let name = name_or_panic(target);
            errors.push(format!(
                "the target `{}` is a binary and can't have any \
                     crate-types set (currently \"{}\")",
                name,
                crate_types.join(", ")
            ));
        } else {
            validate_crate_types(target, "binary", edition, warnings)?;
        }
    }
    Ok(())
}

fn validate_crate_types(
    target: &TomlTarget,
    kind: &str,
    edition: Edition,
    warnings: &mut Vec<String>,
) -> CargoResult<()> {
    deprecated_underscore(
        &target.crate_type2,
        &target.crate_type,
        "crate-type",
        name_or_panic(target),
        format!("{kind} target").as_str(),
        edition,
        warnings,
    )
}

}