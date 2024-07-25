use std::collections::{BTreeMap, HashMap};
use std::io::Seek;
use std::path::{Path, PathBuf};

use anyhow::Context;
use cargo_util_schemas::manifest::{self, StringOrBool};
use clap::{Parser, Subcommand};
use colored::Colorize;

use crate::commands::time::format_duration;
use crate::context::HeatCliContext;
use crate::logging::print_warn;
use crate::{cli_commands, print_debug, print_err, print_info, print_warn, util};

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
pub struct CliArgs {
    #[clap(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
#[command(arg_required_else_help = true)]
pub enum Commands {
    /// {local|remote} : Run a training or inference locally or trigger a remote run.
    #[command(subcommand)]
    Run(cli_commands::run::RunLocationType),

    Package,
    // todo
    // Ls(),
    // todo
    // Login,
    // todo
    // Logout,
}

struct ArchiveFile {
    rel_path: std::path::PathBuf,
    contents: FileContents,
}

enum FileContents {
    /// Absolute path to a file on disk
    OnDisk(PathBuf),
    Generated(GeneratedFile)
}

enum GeneratedFile {
    /// Generates `Cargo.toml` by rewriting the original.
    Manifest,
    /// Generates `Cargo.lock` in some cases (like if there is a binary).
    Lockfile,
    // /// Adds a `.cargo_vcs_info.json` file if in a (clean) git repo.
    // VcsInfo(VcsInfo),
}


fn find_pkg_all_local_dependencies_pkgs(
    root_dir: &Path,
    pkg: &cargo_metadata::Package,
    metadata: &cargo_metadata::Metadata,
) -> anyhow::Result<Vec<cargo_metadata::Package>> {
    let mut deps = Vec::new();
    let mut deps_to_check = vec![pkg];
    while let Some(pkg) = deps_to_check.pop() {
        let pkg_id = cargo_util_schemas::core::PackageIdSpec::parse(&pkg.id.repr).unwrap();
        let pkg_id = pkg_id.to_string();
        let pkg_deps = metadata
            .resolve
            .as_ref()
            .unwrap()
            .nodes
            .iter()
            .find(|node| node.id.repr == pkg_id)
            .unwrap();
        for dep_id in &pkg_deps.dependencies {
            let dep_pkg = metadata
                .packages
                .iter()
                .find(|pkg| pkg.id.repr == dep_id.repr)
                .unwrap();
            if !deps.contains(dep_pkg) {
                // only collect deps that are local (path)
                if let Err(e) = check_package(root_dir, metadata, dep_pkg) {
                    print_err!("Error checking package: {:?}", e);
                    return Err(e);
                }
                if matches!(cargo_util_schemas::core::PackageIdSpec::parse(&dep_pkg.id.repr).unwrap().kind().unwrap(), cargo_util_schemas::core::SourceKind::Path) {
                    deps.push(dep_pkg.clone());
                    deps_to_check.push(dep_pkg);
                }
            }
        }
    }
    Ok(deps)
}

fn check_package(root_dir: &Path, metadata: &cargo_metadata::Metadata, package: &cargo_metadata::Package) -> anyhow::Result<()> {
    use cargo_util_schemas::core::GitReference;
    use cargo_util_schemas::core::PackageIdSpec;
    use cargo_util_schemas::core::SourceKind;

    let cargo_pkgid = PackageIdSpec::parse(&package.id.repr).unwrap();
    let str = match cargo_pkgid.kind().unwrap() {
        SourceKind::Git(git_reference) => match git_reference {
            GitReference::Branch(branch) => {
                format!("Git package: {} {}", package.name, branch)
            }
            GitReference::Tag(tag) => {
                format!("Git package: {} {}", package.name, tag)
            }
            GitReference::Rev(rev) => {
                format!("Git package: {} {}", package.name, rev)
            }
            GitReference::DefaultBranch => {
                format!("Git package: {}", package.name)
            }
        },
        SourceKind::LocalRegistry => {
            format!("Local registry package: {}", package.name)
        }
        SourceKind::Path => {
            format!("Path package: {}", package.name)
        }
        SourceKind::Registry => {
            format!("Registry package: {}", package.name)
        }
        SourceKind::SparseRegistry => {
            format!("Sparse registry package: {}", package.name)
        }
        SourceKind::Directory => {
            format!("Directory package: {}", package.name)
        }
    };

    print_info!("Checking {}", package.name.bold());

    let is_local = matches!(cargo_pkgid.kind().unwrap(), SourceKind::Path);
    if is_local {
        let url = cargo_pkgid.url().unwrap();
        let file_path = url.to_file_path().unwrap();

        let pkg_manifest_dir = package.manifest_path.to_path_buf();
        let pkg_manifest_dir = pkg_manifest_dir.canonicalize().unwrap();

        let is_in_root = pkg_manifest_dir.starts_with(root_dir);
        if is_in_root {
            // check if file exists
            let file_exists = file_path.exists();
            if file_exists {
                // print_info!("Local package: {} at {}", str, pkg_manifest_dir.display());
                // print_info!("Ok")
            } else {
                // print_err!("Package {} is not downloadable", package.name.bold());
                return Err(anyhow::anyhow!("Package {} is not downloadable", package.name));
            }

        } else {
            // print_err!(
            //     "Local package is not in root: {} at {}",
            //     str.bold(),
            //     pkg_manifest_dir.display()
            // );
            return Err(anyhow::anyhow!(
                "Local package is not in root: {} at {}",
                package.name,
                pkg_manifest_dir.display()
            ));
        }
    } else {
        // print_info!("{}", str);

        // let url = cargo_pkgid.url().unwrap();

        // let mut ez = curl::easy::Easy::new();
        // ez.url(url.as_str()).unwrap();
        // ez.nobody(true).unwrap();
        // ez.perform().unwrap();
        // let response_code = ez.response_code().unwrap();
        // if response_code == 200 {
        //     // print_info!("Package {} is downloadable", package.name);
        // } else {
        //     print_err!("Package {} is not downloadable: {}", package.name.bold(), response_code);
        // }
    }

    Ok(())
}

fn package() -> anyhow::Result<()> {
    let cmd = cargo_metadata::MetadataCommand::new();

    let metadata = cmd.exec().expect("Failed to get cargo metadata");

    let own_pkg_name = std::env::var("CARGO_PKG_NAME").expect("CARGO_PKG_NAME not set");
    let root_dir = metadata
        .workspace_root
        .canonicalize()
        .expect("Failed to canonicalize root dir");
    let workspace_toml_path = root_dir.join("Cargo.toml");

    let own_pkg = metadata
        .packages
        .iter()
        .find(|pkg| pkg.name == own_pkg_name)
        .cloned().expect("Failed to find own package");

    let workspace_toml = toml::from_str::<cargo_util_schemas::manifest::TomlManifest>(
        &std::fs::read_to_string(&workspace_toml_path).expect("Failed to read workspace toml"),
    ).expect("Failed to parse workspace toml");

    let workspace_toml = util::toml::read_manifest(&workspace_toml_path, &workspace_toml, None)?;

    // pretty print the workspace toml
    print_debug!("{}", toml::to_string_pretty(&workspace_toml.manifest).unwrap());

    print_info!("{}", "Checking local packages".green().bold());

    // find all local dependencies
    let deps = find_pkg_all_local_dependencies_pkgs(&root_dir, &own_pkg, &metadata)?;
    let pkgs = [vec![own_pkg], deps].concat();

    print_info!("{}", format!("{}", format!("Resolved local dependencies ({})", pkgs.len()).green().bold()));
    for dep in &pkgs {
        print_info!("  {}", dep.name);
    }
    
    let mut dsts = Vec::with_capacity(pkgs.len());

    print_info!("{}", "Archiving project".green().bold());
    for pkg in &pkgs {
        print_info!("  {} {}", "Packaging".green().bold(), pkg.name);
        let pkg_dir = pkg.manifest_path.parent().unwrap();
        let archive_files = prepare_archive(pkg_dir.as_std_path())?;
        for file in &archive_files {
            print_info!("    {}", file.rel_path.display());
        }

        let tarball = create_package(&metadata, pkg, archive_files, &workspace_toml.manifest)?;
        dsts.push(tarball);
    }

    // TODO publish to registry once everything is working

    Ok(())
}

/// Heavily based on cargo's prepare_archive function
fn prepare_archive(root: &Path) -> anyhow::Result<Vec<ArchiveFile>> {

    // here cargo would verify the package metadata

    let files = list_files(root)?;

    // here cargo would check the git repo state

    let archive_files = build_ar_list(root, files)?;

    Ok(archive_files)
}

/// Heavily based on cargo's build_ar_list function
fn build_ar_list(
    pkg_dir: &Path,
    files: Vec<PathBuf>,
) -> anyhow::Result<Vec<ArchiveFile>> {
    const ORIGINAL_MANIFEST_FILENAME: &str = "Cargo.toml.orig";

    let mut result = HashMap::new();
    for file in &files {
        let rel_path = file.strip_prefix(&pkg_dir).unwrap();
        // here cargo would check for filenames that are not allowed
        let rel_str = rel_path.to_str().ok_or_else(|| {
            anyhow::anyhow!("invalid utf-8 in path: {:?}", rel_path)
        })?;
        match rel_str {
            "Cargo.lock" => continue,
            ORIGINAL_MANIFEST_FILENAME => anyhow::bail!(
                "invalid inclusion of reserved file name {} in package source",
                rel_str
            ),
            _ => {
                result
                    .entry(unicase::Ascii::new(rel_str))
                    .or_insert_with(Vec::new)
                    .push(ArchiveFile {
                        rel_path: rel_path.to_owned(),
                        contents: FileContents::OnDisk(file.clone()),
                    });
            }
        }
    }

    if result.remove(&unicase::Ascii::new("Cargo.toml")).is_some() {
        result
            .entry(unicase::Ascii::new(ORIGINAL_MANIFEST_FILENAME))
            .or_insert_with(Vec::new)
            .push(ArchiveFile {
                rel_path: PathBuf::from(ORIGINAL_MANIFEST_FILENAME),
                contents: FileContents::OnDisk(pkg_dir.join("Cargo.toml")),
            });
        result
            .entry(unicase::Ascii::new("Cargo.toml"))
            .or_insert_with(Vec::new)
            .push(ArchiveFile {
                rel_path: PathBuf::from("Cargo.toml"),
                contents: FileContents::Generated(GeneratedFile::Manifest),
            });
    }
    else {
        print_warn!("Cargo.toml not found in package source");
    }

    let pkg_include_lockfile = false;
    if pkg_include_lockfile {
        let rel_str = "Cargo.lock";
        result
            .entry(unicase::Ascii::new(rel_str))
            .or_insert_with(Vec::new)
            .push(ArchiveFile {
                rel_path: PathBuf::from(rel_str),
                contents: FileContents::Generated(GeneratedFile::Lockfile),
            });
    }

    let mut invalid_manifest_field: Vec<String> = Vec::new();

    let mut result: Vec<ArchiveFile> = result.into_values().flatten().collect();
    // here cargo would check for a license file

    // here cargo would check for a readme file

    if !invalid_manifest_field.is_empty() {
        anyhow::bail!(
            "invalid field(s) in manifest: {}",
            invalid_manifest_field.join("\n")
        );
    }

    // todo normalize manifest target paths

    result.sort_unstable_by(|a, b| a.rel_path.cmp(&b.rel_path));

    Ok(result)
}

/// Heavily based on cargo's create_package function
fn create_package(
    metadata: &cargo_metadata::Metadata,
    pkg: &cargo_metadata::Package,
    archive_files: Vec<ArchiveFile>,
    workspace_toml: &cargo_util_schemas::manifest::TomlManifest,
) -> anyhow::Result<std::fs::File> {
    let filecount = archive_files.len();

    // here cargo would check if dependencies have versions and are safe to deploy

    let pkg_id = cargo_util_schemas::core::PackageIdSpec::parse(&pkg.id.repr).unwrap();
    let filename = format!("{}-{}.crate", pkg.name, pkg.version.to_string());
    let dir = metadata.target_directory.join("package");
    std::fs::create_dir_all(&dir)?;

    let tmp = format!(".{}", filename);
    let mut file = std::fs::File::create(dir.join(&tmp))?;
    
    print_info!("Packaging {} files into {}", filecount, filename);
    file.set_len(0)?;
    let uncompressed_size = tar(metadata, pkg, archive_files, &file, &filename, workspace_toml)?;

    file.seek(std::io::SeekFrom::Start(0))?;
    let src_path = &dir.join(&tmp);
    let dst_path = &dir.join(&filename);
    std::fs::rename(src_path, dst_path)?;
    let dst_metadata = file.metadata()?;
    let dst_size = dst_metadata.len();

    /// Formats a number of bytes into a human readable SI-prefixed size.
    /// Returns a tuple of `(quantity, units)`.
    pub fn human_readable_bytes(bytes: u64) -> (f32, &'static str) {
        static UNITS: [&str; 7] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB", "EiB"];
        let bytes = bytes as f32;
        let i = ((bytes.log2() / 10.0) as usize).min(UNITS.len() - 1);
        (bytes / 1024_f32.powi(i as i32), UNITS[i])
    }

    let uncompressed = human_readable_bytes(uncompressed_size);
    let compressed = human_readable_bytes(dst_size);

    let message = format!(
        "{} files, {:.1}{} ({:.1}{} compressed)",
        filecount, uncompressed.0, uncompressed.1, compressed.0, compressed.1,
    );

    print_info!("{} {}", "Packaged".green().bold(), message);

    return Ok(file)
}

/// Heavily based on cargo's tar function
fn tar(
    metadata: &cargo_metadata::Metadata,
    pkg: &cargo_metadata::Package,
    ar_files: Vec<ArchiveFile>,
    dst: &std::fs::File,
    filename: &str,
    workspace_toml: &cargo_util_schemas::manifest::TomlManifest,
) -> anyhow::Result<u64> {

    let filename = Path::new(filename);
    let encoder = flate2::GzBuilder::new()
        .filename(crate::paths::path2bytes(filename)?)
        .write(dst, flate2::Compression::best());

    let mut ar = tar::Builder::new(encoder);

    let base_name = format!("{}-{}", pkg.name, pkg.version.to_string());
    let base_path = Path::new(&base_name);
    let included = ar_files
        .iter()
        .map(|ar_file| ar_file.rel_path.clone())
        .collect::<Vec<_>>();
    let pkg_toml = std::fs::read_to_string(&pkg.manifest_path)?;
    let pkg_toml = toml::from_str::<cargo_util_schemas::manifest::TomlManifest>(&pkg_toml)?;

    let pkg_toml = util::toml::read_manifest(&pkg.manifest_path.as_std_path(), &pkg_toml, Some(&metadata.workspace_root.as_std_path().join("Cargo.toml")))?.manifest;

    print_debug!("{}: {}", pkg.name, 
    toml::to_string_pretty(&pkg_toml).unwrap());

    let publish_toml = prepare_for_publish(metadata, pkg, &pkg_toml, &included, workspace_toml)?;

    let mut uncompressed_size = 0;
    for ar_file in ar_files {
        let ArchiveFile {
            rel_path,
            contents,
        } = ar_file;

        let ar_path = base_path.join(rel_path);
        let mut header = tar::Header::new_gnu();
        match contents {
            FileContents::OnDisk(disk_path) => {
                let mut file = std::fs::File::open(&disk_path)?;
                let metadata = file.metadata()?;
                header.set_metadata_in_mode(&metadata, tar::HeaderMode::Deterministic);
                header.set_cksum();
                ar.append_data(&mut header, &ar_path, &mut file)?;
                uncompressed_size += metadata.len() as u64;
            },
            FileContents::Generated(generated_kind) => {
                let contents = match generated_kind {
                    GeneratedFile::Manifest => toml::to_string_pretty(&publish_toml)?,
                    GeneratedFile::Lockfile => "".to_string(),
                };
                header.set_entry_type(tar::EntryType::file());
                header.set_mode(0o644);
                header.set_size(contents.len() as u64);
                // use something nonzero to avoid rust-lang/cargo#9512
                header.set_mtime(1);
                header.set_cksum();
                ar.append_data(&mut header, &ar_path, contents.as_bytes())?;
                uncompressed_size += contents.len() as u64;
            },
        }
    }

    let encoder = ar.into_inner()?;
    encoder.finish()?;
    Ok(uncompressed_size)
}

pub fn prepare_for_publish(
    metadata: &cargo_metadata::Metadata,
    meta_package: &cargo_metadata::Package,
    pkg_toml: &cargo_util_schemas::manifest::TomlManifest,
    included: &[PathBuf],
    workspace: &cargo_util_schemas::manifest::TomlManifest,
) -> anyhow::Result<cargo_util_schemas::manifest::TomlManifest> {
    let package_root = meta_package.manifest_path.parent().unwrap().as_std_path();

    let mut package = pkg_toml.package().unwrap().clone();
    package.workspace = None;

    if let Some(original_package) = pkg_toml.package() {
        // package
        //         .edition
        //         .as_ref()
        //         .and_then(|e| e.as_value())
        //         .map(|e| Edition::from_str(e))
        //         .unwrap_or(Ok(Edition::Edition2015))
        //         .map(|e| e.default_resolve_behavior())
        // resolve all workspace fields using the workspace 

        let edition = match original_package.resolved_edition() {
            Ok(maybe_edition) => maybe_edition.cloned().unwrap_or_else(|| {
                "2015".to_string()
            }),
            // Edition inherited from workspace 
            Err(..) => match &workspace.workspace.as_ref().unwrap().package.as_ref().unwrap().edition {
                Some(edition) => edition.clone(),
                _ => "2015".to_string(),
            },
        };
        package.edition = Some(cargo_util_schemas::manifest::InheritableField::Value(edition));

        if let Some(license_file) = &package.license_file {
            let license_file = license_file
                .as_value()
                .context("license file should have been resolved before `prepare_for_publish()`")?;
            let license_path = Path::new(&license_file);
            let abs_license_path = crate::paths::normalize_path(&package_root.join(license_path));
            if let Ok(license_file) = abs_license_path.strip_prefix(package_root) {
                package.license_file = Some(manifest::InheritableField::Value(
                    crate::paths::normalize_path_string_sep(
                        license_file
                            .to_str()
                            .ok_or_else(|| anyhow::format_err!("non-UTF8 `package.license-file`"))?
                            .to_owned(),
                    ),
                ));
            } else {
                // This path points outside of the package root. `cargo package`
                // will copy it into the root, so adjust the path to this location.
                package.license_file = Some(manifest::InheritableField::Value(
                    license_path
                        .file_name()
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .to_string(),
                ));
            }
        }
    }

    if let Some(readme) = &package.readme {
        let readme = readme
            .as_value()
            .context("readme should have been resolved before `prepare_for_publish()`")?;
        match readme {
            manifest::StringOrBool::String(readme) => {
                let readme_path = Path::new(&readme);
                let abs_readme_path = crate::paths::normalize_path(&package_root.join(readme_path));
                if let Ok(readme_path) = abs_readme_path.strip_prefix(package_root) {
                    package.readme = Some(manifest::InheritableField::Value(StringOrBool::String(
                        crate::paths::normalize_path_string_sep(
                            readme_path
                                .to_str()
                                .ok_or_else(|| {
                                    anyhow::format_err!("non-UTF8 `package.license-file`")
                                })?
                                .to_owned(),
                        ),
                    )));
                } else {
                    // This path points outside of the package root. `cargo package`
                    // will copy it into the root, so adjust the path to this location.
                    package.readme = Some(manifest::InheritableField::Value(
                        manifest::StringOrBool::String(
                            readme_path
                                .file_name()
                                .unwrap()
                                .to_str()
                                .unwrap()
                                .to_string(),
                        ),
                    ));
                }
            }
            manifest::StringOrBool::Bool(_) => {}
        }
    }

    let lib = if let Some(lib) = &pkg_toml.lib {
        print_debug!("preparing target for publish2: {:?}", lib);

        prepare_target_for_publish(&lib, included, "library")?
    } else {
        None
    };

    let bin = prepare_targets_for_publish(pkg_toml.bin.as_ref(), included, "binary")?;

    let all = |_d: &manifest::TomlDependency| true;
    let resolved_toml = cargo_util_schemas::manifest::TomlManifest {
        cargo_features: pkg_toml.cargo_features.clone(),
        package: Some(package),
        project: pkg_toml.project.clone(),
        profile: pkg_toml.profile.clone(),
        lib,
        bin,
        // Ignore examples, tests, and benchmarks
        example: None,
        test: None,
        bench: None,
        dependencies: map_deps(pkg_toml.dependencies.as_ref(), all)?,
        dev_dependencies: None,
        dev_dependencies2: None,
        build_dependencies: map_deps(pkg_toml.build_dependencies(), all)?,
        build_dependencies2: None,
        features: pkg_toml.features.clone(),
        target: pkg_toml.target.clone(),
        replace: pkg_toml.replace.clone(),
        patch: pkg_toml.patch.clone(),
        workspace: None,
        badges: pkg_toml.badges.clone(),
        lints: pkg_toml.lints.clone(),
        _unused_keys: pkg_toml._unused_keys.clone(),
    };
    
    Ok(resolved_toml)
}

fn map_deps(
    deps: Option<&BTreeMap<manifest::PackageName, manifest::InheritableDependency>>,
    filter: impl Fn(&manifest::TomlDependency) -> bool,
) -> anyhow::Result<Option<BTreeMap<manifest::PackageName, manifest::InheritableDependency>>> {
    let Some(deps) = deps else {
        return Ok(None);
    };
    let deps = deps
        .iter()
        .filter(|(_k, v)| {
            if let manifest::InheritableDependency::Value(def) = v {
                filter(def)
            } else {
                false
            }
        })
        .map(|(k, v)| Ok((k.clone(), map_dependency(v)?)))
        .collect::<anyhow::Result<BTreeMap<_, _>>>()?;
    Ok(Some(deps))
}

fn map_dependency(
    dep: &manifest::InheritableDependency,
) -> anyhow::Result<manifest::InheritableDependency> {
    let dep = match dep {
        manifest::InheritableDependency::Value(manifest::TomlDependency::Detailed(d)) => {
            let mut d = d.clone();
            // Path dependencies become crates.io deps.
            d.path.take();
            // Same with git dependencies.
            // d.git.take();
            // d.branch.take();
            // d.tag.take();
            // d.rev.take();
            // registry specifications are elaborated to the index URL
            // if let Some(registry) = d.registry.take() {
            //     d.registry_index = Some(get_registry_index(&registry)?.to_string());
            // }
            Ok(d)
        }
        manifest::InheritableDependency::Value(manifest::TomlDependency::Simple(s)) => {
            Ok(manifest::TomlDetailedDependency {
                version: Some(s.clone()),
                ..Default::default()
            })
        }
        _ => unreachable!(),
    };
    dep.map(manifest::TomlDependency::Detailed)
        .map(manifest::InheritableDependency::Value)
}

fn prepare_target_for_publish(
    target: &manifest::TomlTarget,
    included: &[PathBuf],
    context: &str,
) -> anyhow::Result<Option<manifest::TomlTarget>> {
    let path = target.path.as_ref().expect(format!("previously resolved {:?} path", target).as_str());
    let path = crate::paths::normalize_path(&path.0);
    if !included.contains(&path) {
        let name = target.name.as_ref().expect("previously resolved");
        print_warn!("{}", format!(
            "ignoring {context} `{name}` as `{}` is not included in the published package",
            path.display()
        ));
        return Ok(None);
    }

    let mut target = target.clone();
    let path = crate::paths::normalize_path_sep(path, context)?;
    target.path = Some(manifest::PathValue(path.into()));

    Ok(Some(target))
}

fn prepare_targets_for_publish(
    targets: Option<&Vec<manifest::TomlTarget>>,
    included: &[PathBuf],
    context: &str,
) -> anyhow::Result<Option<Vec<manifest::TomlTarget>>> {
    let Some(targets) = targets else {
        return Ok(None);
    };

    let mut prepared = Vec::with_capacity(targets.len());
    for target in targets {
        print_debug!("preparing target for publish: {:?}", target);
        let Some(target) = prepare_target_for_publish(target, included, context)? else {
            continue;
        };
        prepared.push(target);
    }

    if prepared.is_empty() {
        Ok(None)
    } else {
        Ok(Some(prepared))
    }
}


/// Heavily based on cargo's list_files function
fn list_files(pkg_path: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let excludes: Vec<&str> = vec![];
    let includes: Vec<&str> = vec![];

    let no_include_option = includes.is_empty();
    let git_repo = if no_include_option {
        discover_gix_repo(pkg_path)?
    } else {
        None
    };

    if let Some(repo) = &git_repo {
        print_info!("Git repo found at {}", repo.path().display().to_string().bold());
    }

    let mut exclude_builder = ignore::gitignore::GitignoreBuilder::new(pkg_path);
    if no_include_option && git_repo.is_none() {
        exclude_builder.add_line(None, ".*")?;
    }
    for rule in excludes {
        exclude_builder.add_line(None, rule)?;
    }
    let ignore_exclude = exclude_builder.build()?;

    let mut include_builder = ignore::gitignore::GitignoreBuilder::new(pkg_path);
    for rule in includes {
        include_builder.add_line(None, rule)?;
    }
    let ignore_include = include_builder.build()?;

    let ignore_should_package = |relative_path: &Path, is_dir: bool| {
        // "Include" and "exclude" options are mutually exclusive.
        if no_include_option {
            !ignore_exclude
                .matched_path_or_any_parents(relative_path, is_dir)
                .is_ignore()
        } else {
            if is_dir {
                // Generally, include directives don't list every
                // directory (nor should they!). Just skip all directory
                // checks, and only check files.
                return true;
            }
            ignore_include
                .matched_path_or_any_parents(relative_path, /* is_dir */ false)
                .is_ignore()
        }
    };

    let include_lockfile = false;
    let filter = |path: &Path, is_dir: bool| {
        let Ok(relative_path) = path.strip_prefix(pkg_path) else {
            return false;
        };

        let rel = relative_path.as_os_str();
        if rel == "Cargo.lock" {
            return include_lockfile;
        } else if rel == "Cargo.toml" {
            return true;
        }

        ignore_should_package(relative_path, is_dir)
    };

    // Attempt Git-prepopulate only if no `include` (see rust-lang/cargo#4135).
    if no_include_option {
        if let Some(repo) = git_repo {
            return list_files_gix(pkg_path, &repo, &filter);
        }
    }
    let mut ret = Vec::new();
    list_files_walk(pkg_path, &mut ret, true, &filter)?;
    Ok(ret)
}

/// Taken from cargo's list_files_walk function
/// 
/// Returns [`Some(gix::Repository)`](gix::Repository) if the discovered repository
/// (searched upwards from `root`) contains a tracked `<root>/Cargo.toml`.
/// Otherwise, the caller should fall back on full file list.
fn discover_gix_repo(root: &Path) -> anyhow::Result<Option<gix::Repository>> {
    let repo = match gix::ThreadSafeRepository::discover(root) {
        Ok(repo) => repo.to_thread_local(),
        Err(e) => {
            // tracing::debug!(
            //     "could not discover git repo at or above {}: {}",
            //     root.display(),
            //     e
            // );
            return Ok(None);
        }
    };
    let index = repo.index_or_empty()?;
    // .with_context(|| format!("failed to open git index at {}", repo.path().display()))?;
    let repo_root = repo.work_dir().ok_or_else(|| {
        anyhow::format_err!(
            "did not expect repo at {} to be bare",
            repo.path().display()
        )
    })?;

    /// Strips `base` from `path`.
    ///
    /// This canonicalizes both paths before stripping. This is useful if the
    /// paths are obtained in different ways, and one or the other may or may not
    /// have been normalized in some way.
    pub fn strip_prefix_canonical<P: AsRef<Path>>(
        path: P,
        base: P,
    ) -> Result<PathBuf, std::path::StripPrefixError> {
        // Not all filesystems support canonicalize. Just ignore if it doesn't work.
        let safe_canonicalize = |path: &Path| match path.canonicalize() {
            Ok(p) => p,
            Err(e) => {
                print_warn!("cannot canonicalize {:?}: {:?}", path, e);
                path.to_path_buf()
            }
        };
        let canon_path = safe_canonicalize(path.as_ref());
        let canon_base = safe_canonicalize(base.as_ref());
        canon_path.strip_prefix(canon_base).map(|p| p.to_path_buf())
    }

    let repo_relative_path = match strip_prefix_canonical(root, repo_root) {
        Ok(p) => p,
        Err(e) => {
            print_warn!(
                "cannot determine if path `{:?}` is in git repo `{:?}`: {:?}",
                root,
                repo_root,
                e
            );
            return Ok(None);
        }
    };
    let manifest_path = gix::path::join_bstr_unix_pathsep(
        gix::path::to_unix_separators_on_windows(gix::path::into_bstr(repo_relative_path)),
        "Cargo.toml",
    );
    if index.entry_index_by_path(&manifest_path).is_ok() {
        return Ok(Some(repo));
    }
    // Package Cargo.toml is not in git, don't use git to guide our selection.
    Ok(None)
}

/// Taken from cargo's list_files_walk function
/// 
/// Lists files relevant to building this package inside this source by
/// traversing the git working tree, while avoiding ignored files.
///
/// This looks into Git sub-repositories as well, resolving them to individual files.
/// Symlinks to directories will also be resolved, but walked as repositories if they
/// point to one to avoid picking up `.git` directories.
fn list_files_gix(
    pkg_path: &Path,
    repo: &gix::Repository,
    filter: &dyn Fn(&Path, bool) -> bool,
) -> anyhow::Result<Vec<PathBuf>> {
    let options = repo
        .dirwalk_options()?
        .emit_untracked(gix::dir::walk::EmissionMode::Matching)
        .emit_ignored(None)
        .emit_tracked(true)
        .recurse_repositories(false)
        .symlinks_to_directories_are_ignored_like_directories(true)
        .emit_empty_directories(false);

    let index = repo.index_or_empty()?;
    let root = repo
        .work_dir()
        .ok_or_else(|| anyhow::anyhow!("No work dir"))?;
    assert!(root.is_absolute(), "Work dir is not absolute");

    let repo_relative_pkg_path = pkg_path.strip_prefix(root).unwrap_or(Path::new(""));
    let target_prefix = gix::path::to_unix_separators_on_windows(gix::path::into_bstr(
        repo_relative_pkg_path.join("target/"),
    ));
    let package_prefix =
        gix::path::to_unix_separators_on_windows(gix::path::into_bstr(repo_relative_pkg_path));

    use gix::bstr::{BString, ByteVec};
    use gix::dir::entry::Status;
    use gix::index::entry::Stage;

    let pathspec = {
        // Include the package root.
        let mut include = BString::from(":/");
        include.push_str(package_prefix.as_ref());

        // Exclude the target directory.
        let mut exclude = BString::from(":!/");
        exclude.push_str(target_prefix.as_ref());

        vec![include, exclude]
    };

    let mut files = Vec::<PathBuf>::new();
    let mut subpackages_found = Vec::new();

    let filtered_repo = repo
        .dirwalk_iter(index.clone(), pathspec, Default::default(), options)?
        .filter(|res| {
            res.as_ref().map_or(true, |item| {
                !(item.entry.status == Status::Untracked && item.entry.rela_path == "Cargo.lock")
            })
        })
        .map(|res| res.map(|item| (item.entry.rela_path, item.entry.disk_kind)))
        .chain(
            // Append entries that might be tracked in `<pkg_root>/target/`.
            index
                .prefixed_entries(target_prefix.as_ref())
                .unwrap_or_default()
                .iter()
                .filter(|entry| {
                    // probably not needed as conflicts prevent this to run, but let's be explicit.
                    entry.stage() == Stage::Unconflicted
                })
                .map(|entry| {
                    (
                        entry.path(&index).to_owned(),
                        // Do not trust what's recorded in the index, enforce checking the disk.
                        // This traversal is not part of a `status()`, and tracking things in `target/`
                        // is rare.
                        None,
                    )
                })
                .map(Ok),
        );

    for item in filtered_repo {
        let (rela_path, kind) = item?;
        let file_path = root.join(gix::path::from_bstr(rela_path));
        if file_path.file_name().and_then(|name| name.to_str()) == Some("Cargo.toml") {
            // Keep track of all sub-packages found and also strip out all
            // matches we've found so far. Note, though, that if we find
            // our own `Cargo.toml`, we keep going.
            let path = file_path.parent().unwrap();
            if path != pkg_path {
                print_info!("subpackage found: {}", path.display());
                files.retain(|p| !p.starts_with(path));
                subpackages_found.push(path.to_path_buf());
                continue;
            }
        }

        // If this file is part of any other sub-package we've found so far,
        // skip it.
        if subpackages_found.iter().any(|p| file_path.starts_with(p)) {
            continue;
        }

        let is_dir = kind.map_or(false, |kind| {
            if kind == gix::dir::entry::Kind::Symlink {
                // Symlinks must be checked to see if they point to a directory
                // we should traverse.
                file_path.is_dir()
            } else {
                kind.is_dir()
            }
        });
        if is_dir {
            // This could be a submodule, or a sub-repository. In any case, we prefer to walk
            // it with git-support to leverage ignored files and to avoid pulling in entire
            // .git repositories.
            match gix::open(&file_path) {
                Ok(sub_repo) => {
                    files.extend(list_files_gix(pkg_path, &sub_repo, filter)?);
                }
                Err(_) => {
                    list_files_walk(&file_path, &mut files, false, filter)?;
                }
            }
        } else if (filter)(&file_path, is_dir) {
            assert!(!is_dir);
            // print_info!("  found {}", file_path.display());
            files.push(file_path);
        }
    }

    return Ok(files);
}

/// Heavily based on cargo's list_files_walk function
/// 
/// Lists files relevant to building this package inside this source by
/// walking the filesystem from the package root path.
///
/// This is a fallback for [`list_files_gix`] when the package
/// is not tracked under a Git repository.
fn list_files_walk(
    path: &Path,
    ret: &mut Vec<PathBuf>,
    is_root: bool,
    filter: &dyn Fn(&Path, bool) -> bool,
) -> anyhow::Result<()> {
    use walkdir::WalkDir;
    let walkdir = WalkDir::new(path)
        .follow_links(true)
        .into_iter()
        .filter_entry(|entry| {
            let path = entry.path();
            let at_root = is_root && entry.depth() == 0;
            let is_dir = entry.file_type().is_dir();

            if !at_root && !filter(path, is_dir) {
                return false;
            }

            if !is_dir {
                return true;
            }

            // Don't recurse into any sub-packages that we have.
            if !at_root && path.join("Cargo.toml").exists() {
                return false;
            }

            // Skip root Cargo artifacts.
            if is_root
                && entry.depth() == 1
                && path.file_name().and_then(|s| s.to_str()) == Some("target")
            {
                return false;
            }

            true
        });
    for entry in walkdir {
        match entry {
            Ok(entry) => {
                if !entry.file_type().is_dir() {
                    ret.push(entry.into_path());
                }
            }
            Err(err) if err.loop_ancestor().is_some() => {
                print_warn!("{}", err);
            }
            Err(err) => match err.path() {
                // If an error occurs with a path, filter it again.
                // If it is excluded, Just ignore it in this case.
                // See issue rust-lang/cargo#10917
                Some(path) if !filter(path, path.is_dir()) => {}
                // Otherwise, simply recover from it.
                // Don't worry about error skipping here, the callers would
                // still hit the IO error if they do access it thereafter.
                Some(path) => ret.push(path.to_path_buf()),
                None => return Err(err.into()),
            },
        }
    }

    Ok(())
}

pub fn cli_main() {
    print_info!("Running CLI.");
    let time_begin = std::time::Instant::now();
    let args = CliArgs::try_parse();
    if args.is_err() {
        print_err!("{}", args.unwrap_err());
        std::process::exit(1);
    }

    let user_project_name = std::env::var("CARGO_PKG_NAME").expect("CARGO_PKG_NAME not set");
    let user_crate_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set");

    let context = HeatCliContext::new(user_project_name, user_crate_dir.into()).init();

    let cli_res = match args.unwrap().command {
        Commands::Run(run_args) => cli_commands::run::handle_command(run_args, context),
        Commands::Package => package(),
    };

    match cli_res {
        Ok(_) => {
            print_info!("CLI command executed successfully.");
        }
        Err(e) => {
            print_err!("Error executing CLI command: {:?}", e);
        }
    }

    let duration = time_begin.elapsed();
    print_info!(
        "\x1B[32;1mTime elapsed for the current execution: {}\x1B[0m",
        format_duration(&duration)
    );
}
