
use std::{collections::{BTreeMap, HashSet}, ffi::OsStr, path::{Path, PathBuf}, str::FromStr};

use lazycell::LazyCell;

use cargo_metadata::{semver, PackageId};
use cargo_util_schemas::manifest::{self, RustVersion, StringOrBool, TomlManifest};

type CargoResult<T> = anyhow::Result<T>;

use anyhow::{bail, Context};

use crate::print_debug;

use super::{dependency::{DepKind, FeatureValue}, features::Edition, interning::InternedString, workspace::{resolve_relative_path, WorkspaceConfig, WorkspaceRootConfig}};

/// See also `bin/cargo/commands/run.rs`s `is_manifest_command`
pub fn is_embedded(path: &Path) -> bool {
    let ext = path.extension();
    ext == Some(OsStr::new("rs")) ||
        // Provide better errors by not considering directories to be embedded manifests
        (ext.is_none() && path.is_file())
}

pub struct Workspace {
    pub manifest: manifest::TomlManifest,
    pub root: PathBuf,
    pub config: WorkspaceConfig,
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
    original_toml: &manifest::TomlManifest,
    root_workspace_path: Option<&Path>
) -> CargoResult<Workspace> {

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
    )?;

    Ok(Workspace {
        manifest: resolved_toml,
        root: path.to_owned(),
        config: workspace_config,
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
        readme: None,
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
            .map_or(Edition::default(), |e| Edition::from_str(&e.unwrap()).unwrap_or_default());
        resolved_toml.package = Some(resolved_package);

        resolved_toml.features = original_toml.features.clone();//resolve_features(original_toml.features.as_ref())?;

        // resolved_toml.lib = targets::resolve_lib(
        //     original_toml.lib.as_ref(),
        //     package_root,
        //     &original_package.name,
        //     edition,
        //     warnings,
        // )?;
        // resolved_toml.bin = Some(targets::resolve_bins(
        //     original_toml.bin.as_ref(),
        //     package_root,
        //     &original_package.name,
        //     edition,
        //     original_package.autobins,
        //     warnings,
        //     errors,
        //     resolved_toml.lib.is_some(),
        // )?);

        resolved_toml.lib = original_toml.lib.clone();
        resolved_toml.bin = original_toml.bin.clone();

        // resolved_toml.example = Some(targets::resolve_examples(
        //     original_toml.example.as_ref(),
        //     package_root,
        //     edition,
        //     original_package.autoexamples,
        //     warnings,
        //     errors,
        // )?);
        // resolved_toml.test = Some(targets::resolve_tests(
        //     original_toml.test.as_ref(),
        //     package_root,
        //     edition,
        //     original_package.autotests,
        //     warnings,
        //     errors,
        // )?);
        // resolved_toml.bench = Some(targets::resolve_benches(
        //     original_toml.bench.as_ref(),
        //     package_root,
        //     edition,
        //     original_package.autobenches,
        //     warnings,
        //     errors,
        // )?);
        let warnings = &mut Vec::new();
    
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
            if d.public.is_some() {
                let public_feature: CargoResult<()> = Ok(());//features.require(Feature::public_dependency());
                let with_public_feature = public_feature.is_ok();
                let with_z_public = false;//gctx.cli_unstable().public_dependency;
                if !with_public_feature && (!with_z_public) {
                    public_feature?;
                }
                if matches!(kind, None) {
                    if !with_public_feature && !with_z_public {
                        d.public = None;
                        warnings.push(format!(
                            "ignoring `public` on dependency {name_in_toml}, pass `-Zpublic-dependency` to enable support for it"
                        ))
                    }
                } else {
                    let kind_name = match kind {
                        Some(k) => k.kind_table(),
                        None => "dependencies",
                    };
                    let hint = format!(
                        "'public' specifier can only be used on regular dependencies, not {kind_name}",
                    );
                    if with_public_feature || with_z_public {
                        bail!(hint)
                    } else {
                        // If public feature isn't enabled in nightly, we instead warn that.
                        warnings.push(hint);
                        d.public = None;
                    }
                }
            }
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

    let toml = std::fs::read_to_string(&workspace_path).with_context(|| {
        format!("failed to read manifest at `{}`", workspace_path.display())
    })?;
    let original_toml = toml::from_str::<TomlManifest>(&toml)?;

    // let source_id = SourceId::for_path(workspace_path_root)?;
    let man = read_manifest(&workspace_path, &original_toml, root_workspace_path)?;
    match man.config {
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

    // /// Gets the field `workspace.package.readme`.
    // fn readme(&self, package_root: &Path) -> CargoResult<manifest::StringOrBool> {
    //     let Some(readme) = resolve_package_readme(
    //         self._ws_root.as_path(),
    //         self.package.as_ref().and_then(|p| p.readme.as_ref()),
    //     ) else {
    //         bail!("`workspace.package.readme` was not defined");
    //     };
    //     resolve_relative_path("readme", &self._ws_root, package_root, &readme)
    //         .map(manifest::StringOrBool::String)
    // }

    fn ws_root(&self) -> &PathBuf {
        &self._ws_root
    }
}