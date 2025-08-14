use std::{
    collections::{BTreeMap, HashMap},
    io::Seek,
    path::{Path, PathBuf},
};

use burn_central_client::schemas::{CrateMetadata, Dep, PackagedCrateData};
use colored::Colorize;

use super::paths;
use crate::{print_err, print_info, print_warn, util};
use sha2::Digest as _;
use sha2::Sha256;

/// Based on the struct ArchiveFile from Cargo: https://github.com/rust-lang/cargo/blob/57622d793935a662b5f14ca728a2989c14833d37/src/cargo/ops/cargo_package.rs#L48
/// rel_str member was removed
struct ArchiveFile {
    rel_path: std::path::PathBuf,
    contents: FileContents,
}

/// From Cargo: https://github.com/rust-lang/cargo/blob/57622d793935a662b5f14ca728a2989c14833d37/src/cargo/ops/cargo_package.rs#L58
enum FileContents {
    /// Absolute path to a file on disk
    OnDisk(PathBuf),
    Generated(GeneratedFile),
}

/// Based on the enum GeneratedFile from Cargo: https://github.com/rust-lang/cargo/blob/57622d793935a662b5f14ca728a2989c14833d37/src/cargo/ops/cargo_package.rs#L65
/// VcsInfo variant was removed
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
                if let Err(e) = check_package(root_dir, dep_pkg) {
                    print_err!("Error checking package: {:?}", e);
                    // return Err(e);
                }
                else if matches!(
                    cargo_util_schemas::core::PackageIdSpec::parse(&dep_pkg.id.repr)
                        .unwrap()
                        .kind()
                        .unwrap(),
                    cargo_util_schemas::core::SourceKind::Path
                ) {
                    deps.push(dep_pkg.clone());
                    deps_to_check.push(dep_pkg);
                }
            }
        }
    }
    Ok(deps)
}

fn check_package(root_dir: &Path, package: &cargo_metadata::Package) -> anyhow::Result<()> {
    use cargo_util_schemas::core::PackageIdSpec;
    use cargo_util_schemas::core::SourceKind;

    let cargo_pkgid = PackageIdSpec::parse(&package.id.repr).unwrap();

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
                return Err(anyhow::anyhow!(
                    "Package {} is not downloadable",
                    package.name
                ));
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
    }

    Ok(())
}

pub struct Package {
    pub package: cargo_metadata::Package,
    pub manifest: util::cargo::toml::Manifest,
    pub manifest_path: PathBuf,
}

pub fn package(
    artifacts_dir: &Path,
    target_package_name: &str,
) -> anyhow::Result<Vec<PackagedCrateData>> {
    let cmd = cargo_metadata::MetadataCommand::new();

    let metadata = cmd.exec().expect("Failed to get cargo metadata");

    let own_pkg_name = target_package_name.to_string();
    let root_dir = metadata
        .workspace_root
        .canonicalize()
        .expect("Failed to canonicalize root dir");
    let workspace_toml_path = root_dir.join("Cargo.toml");

    let own_pkg = metadata
        .packages
        .iter()
        .find(|pkg| pkg.name == own_pkg_name.parse().unwrap())
        .cloned()
        .expect("Failed to find own package");

    let workspace_toml =
        util::cargo::toml::read_manifest(&workspace_toml_path, Some(&workspace_toml_path))?;

    print_info!("{}", "Checking local packages".green().bold());

    // find all local dependencies
    let deps = find_pkg_all_local_dependencies_pkgs(&root_dir, &own_pkg, &metadata)?;
    let pkgs = [vec![own_pkg], deps].concat();

    print_info!(
        "{}",
        format!(
            "{}",
            format!("Resolved local dependencies ({})", pkgs.len())
                .green()
                .bold()
        )
    );
    for dep in &pkgs {
        print_info!("  {}", dep.name);
    }

    let mut dsts = Vec::with_capacity(pkgs.len());

    print_info!("{}", "Archiving project".green().bold());

    let mut package_cmd = std::process::Command::new("cargo");
    package_cmd
        .arg("package")
        .arg("-Zpackage-workspace")
        .arg("--no-metadata")
        .args(["--no-verify", "--allow-dirty"])
        .args(["--target-dir", artifacts_dir.to_str().unwrap()])
        .args(pkgs.iter().map(|pkg| format!("-p{}", pkg.name)))
        .env("RUSTC_BOOTSTRAP", "1");

    let package_status = package_cmd
        .status()
        .expect("Failed to run cargo package command");

    if !package_status.success() {
        print_err!("Failed to run cargo package command");
        return Err(anyhow::anyhow!("Failed to run cargo package command"));
    }

    let packaged_artifacts_dir = artifacts_dir.join("package");
    // get all .crate files in the artifacts directory
    let tarballs = std::fs::read_dir(&packaged_artifacts_dir)?
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let path = entry.path();
            if path.extension()?.to_str()? == "crate" {
                let filename = path.file_name()?.to_str()?.to_string();
                let path = packaged_artifacts_dir.join(&filename);
                let dst_path = artifacts_dir.join(&filename);
                std::fs::rename(&path, &dst_path).ok()?;
                let data = std::fs::read(&dst_path).ok()?;
                let checksum = format!("{:x}", Sha256::digest(data));
                Some((filename, dst_path, checksum))
            } else {
                None
            }
        })
        .collect::<Vec<_>>();

    // delete the original package directory
    std::fs::remove_dir_all(&packaged_artifacts_dir)
        .expect("Failed to remove packaged artifacts directory");

    for pkg in &pkgs {
        print_info!("  {} {}", "Packaging".green().bold(), pkg.name);
        // let pkg_dir = pkg.manifest_path.parent().unwrap();
        // let archive_files = prepare_archive(pkg_dir.as_std_path())?;
        // for file in &archive_files {
        //     print_info!("    {}", file.rel_path.display());
        // }

        let crate_deps = pkg
            .dependencies
            .iter()
            .map(|dep| {
                let is_local = dep.path.is_some();
                Dep::new(
                    dep.name.clone(),
                    dep.req.to_string(),
                    dep.features.clone(),
                    dep.optional,
                    dep.uses_default_features,
                    dep.target.clone().map(|t| t.to_string()),
                    match dep.kind {
                        cargo_metadata::DependencyKind::Normal => {
                            burn_central_client::schemas::DepKind::Normal
                        }
                        cargo_metadata::DependencyKind::Development => {
                            burn_central_client::schemas::DepKind::Dev
                        }
                        cargo_metadata::DependencyKind::Build => {
                            burn_central_client::schemas::DepKind::Build
                        }
                        cargo_metadata::DependencyKind::Unknown => {
                            unimplemented!("Unknown dep kind")
                        }
                    },
                    if is_local {
                        None
                    } else {
                        match &dep.registry {
                            chose @ Some(..) => chose.clone(),
                            None => {
                                Some("https://github.com/rust-lang/crates.io-index".to_string())
                            }
                        }
                    },
                    None,
                )
            })
            .collect::<Vec<_>>();
        let crate_metadata = CrateMetadata::new(
            pkg.name.clone().to_string(),
            pkg.version.to_string(),
            crate_deps,
            pkg.features.clone(),
            pkg.authors.clone(),
            pkg.description.clone(),
            pkg.documentation.clone(),
            pkg.homepage.clone(),
            None,
            pkg.readme.clone().map(|r| r.to_string()),
            pkg.keywords.clone(),
            pkg.categories.clone(),
            pkg.license.clone(),
            pkg.license_file.clone().map(|l| l.to_string()),
            pkg.repository.clone(),
            BTreeMap::new(),
            pkg.links.clone(),
        );

        let tarball = tarballs.iter().find(|f| {
            f.0.starts_with(pkg.name.as_str())
        }).ok_or_else(|| {
            anyhow::anyhow!(
                "Failed to find tarball for package {} in {}",
                pkg.name,
                packaged_artifacts_dir.display()
            )
        })?.clone();

        dsts.push(PackagedCrateData {
            name: tarball.0,
            path: tarball.1,
            checksum: tarball.2,
            metadata: crate_metadata,
        });
    }

    Ok(dsts)
}

/// Heavily based on cargo's prepare_archive function: https://github.com/rust-lang/cargo/blob/57622d793935a662b5f14ca728a2989c14833d37/src/cargo/ops/cargo_package.rs#L220
///
/// /// Performs pre-archiving checks and builds a list of files to archive.
fn prepare_archive(root: &Path) -> anyhow::Result<Vec<ArchiveFile>> {
    // here cargo would verify the package metadata

    let files = list_files(root)?;

    // here cargo would check the git repo state

    let archive_files = build_ar_list(root, files)?;

    Ok(archive_files)
}

/// Heavily based on cargo's build_ar_list function: https://github.com/rust-lang/cargo/blob/57622d793935a662b5f14ca728a2989c14833d37/src/cargo/ops/cargo_package.rs#L248
///
/// Builds list of files to archive
fn build_ar_list(pkg_dir: &Path, files: Vec<PathBuf>) -> anyhow::Result<Vec<ArchiveFile>> {
    const ORIGINAL_MANIFEST_FILENAME: &str = "Cargo.toml.orig";

    let mut result = HashMap::new();
    for file in &files {
        let rel_path = file.strip_prefix(pkg_dir).unwrap();
        // here cargo would check for filenames that are not allowed
        let rel_str = rel_path
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("invalid utf-8 in path: {:?}", rel_path))?;
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
    } else {
        print_warn!("Cargo.toml not found in package source");
    }

    // todo : check whether to include lockfile or not
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

    let invalid_manifest_field: Vec<String> = Vec::new();

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

/// Heavily based on cargo's create_package function: https://github.com/rust-lang/cargo/blob/57622d793935a662b5f14ca728a2989c14833d37/src/cargo/ops/cargo_package.rs#L114
///
/// Builds a tarball and places it in the output directory.
fn create_package(
    pkg: Package,
    archive_files: Vec<ArchiveFile>,
    artifacts_dir: &Path,
    workspace_toml: &cargo_util_schemas::manifest::TomlManifest,
) -> anyhow::Result<(String, PathBuf, String)> {
    let filecount = archive_files.len();

    // here cargo would check if dependencies have versions and are safe to deploy

    let filename = format!("{}-{}.crate", pkg.package.name, pkg.package.version);
    let dir = artifacts_dir;
    std::fs::create_dir_all(dir)?;

    let tmp = format!(".{filename}");
    let mut file = std::fs::File::create(dir.join(&tmp))?;

    print_info!("Packaging {} files into {}", filecount, filename);
    file.set_len(0)?;
    let uncompressed_size = tar(&pkg, archive_files, &file, &filename, workspace_toml)?;

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

    let checksum = {
        let data = std::fs::read(dst_path)?;
        format!("{:x}", Sha256::digest(data))
    };

    print_info!("{} {}", "Packaged".green().bold(), message);

    Ok((filename, dst_path.into(), checksum))
}

/// Heavily based on cargo's tar function: https://github.com/rust-lang/cargo/blob/57622d793935a662b5f14ca728a2989c14833d37/src/cargo/ops/cargo_package.rs#L712
///
/// Compresses and packages a list of [`ArchiveFile`]s and writes into the given file.
///
/// Returns the uncompressed size of the contents of the new archive file.
fn tar(
    pkg: &Package,
    ar_files: Vec<ArchiveFile>,
    dst: &std::fs::File,
    filename: &str,
    workspace_toml: &cargo_util_schemas::manifest::TomlManifest,
) -> anyhow::Result<u64> {
    let filename = Path::new(filename);
    let encoder = flate2::GzBuilder::new()
        .filename(paths::path2bytes(filename)?)
        .write(dst, flate2::Compression::best());

    let mut ar = tar::Builder::new(encoder);

    let base_name = format!("{}-{}", pkg.package.name, pkg.package.version);
    let base_path = Path::new(&base_name);
    let included = ar_files
        .iter()
        .map(|ar_file| ar_file.rel_path.clone())
        .collect::<Vec<_>>();

    let publish_toml = super::toml::prepare_toml_for_publish(
        &pkg.manifest.resolved_toml,
        workspace_toml,
        pkg.manifest_path.parent().unwrap(),
        &included,
    )?;

    let mut uncompressed_size: u64 = 0;
    for ar_file in ar_files {
        let ArchiveFile { rel_path, contents } = ar_file;

        let ar_path = base_path.join(rel_path);
        let mut header = tar::Header::new_gnu();
        match contents {
            FileContents::OnDisk(disk_path) => {
                let mut file = std::fs::File::open(&disk_path)?;
                let metadata = file.metadata()?;
                header.set_metadata_in_mode(&metadata, tar::HeaderMode::Deterministic);
                header.set_cksum();
                ar.append_data(&mut header, &ar_path, &mut file)?;
                uncompressed_size += metadata.len();
            }
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
            }
        }
    }

    let encoder = ar.into_inner()?;
    encoder.finish()?;
    Ok(uncompressed_size)
}

/// Heavily based on Cargo's _list_files function: https://github.com/rust-lang/cargo/blob/57622d793935a662b5f14ca728a2989c14833d37/src/cargo/sources/path.rs#L458
fn list_files(pkg_path: &Path) -> anyhow::Result<Vec<PathBuf>> {
    // todo : handle includes and excludes
    // for now everything is left empty
    let excludes: Vec<&str> = vec![];
    let includes: Vec<&str> = vec![];

    let no_include_option = includes.is_empty();
    let git_repo = if no_include_option {
        discover_gix_repo(pkg_path)?
    } else {
        None
    };

    if let Some(repo) = &git_repo {
        print_info!(
            "Git repo found at {}",
            repo.path().display().to_string().bold()
        );
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

/// Taken from cargo's list_files_walk function: https://github.com/rust-lang/cargo/blob/57622d793935a662b5f14ca728a2989c14833d37/src/cargo/sources/path.rs#L531
///
/// Returns [`Some(gix::Repository)`](gix::Repository) if the discovered repository
/// (searched upwards from `root`) contains a tracked `<root>/Cargo.toml`.
/// Otherwise, the caller should fall back on full file list.
fn discover_gix_repo(root: &Path) -> anyhow::Result<Option<gix::Repository>> {
    let repo = match gix::ThreadSafeRepository::discover(root) {
        Ok(repo) => repo.to_thread_local(),
        Err(..) => {
            return Ok(None);
        }
    };
    let index = repo.index_or_empty()?;
    // .with_context(|| format!("failed to open git index at {}", repo.path().display()))?;
    let repo_root = repo.workdir().ok_or_else(|| {
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

/// Based on Cargo's list_files_gix function: https://github.com/rust-lang/cargo/blob/c1fa840a85eca53818895901a53fae34247448b2/src/cargo/sources/path.rs#L579
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
        .workdir()
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

        let is_dir = kind.is_some_and(|kind| {
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

    Ok(files)
}

/// Taken from Cargo's list_files_walk function: https://github.com/rust-lang/cargo/blob/c1fa840a85eca53818895901a53fae34247448b2/src/cargo/sources/path.rs#L714
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
