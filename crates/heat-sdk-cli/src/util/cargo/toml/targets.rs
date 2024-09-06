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
    PathValue, StringOrBool, TomlBenchTarget, TomlBinTarget, TomlExampleTarget, TomlLibTarget,
    TomlTarget, TomlTestTarget,
};

use super::CargoResult;

use crate::util::cargo::features::Edition;
use crate::util::cargo::restricted_names;
use crate::util::cargo::toml::deprecated_underscore;

const DEFAULT_TEST_DIR_NAME: &str = "tests";
const DEFAULT_BENCH_DIR_NAME: &str = "benches";
const DEFAULT_EXAMPLE_DIR_NAME: &str = "examples";

/// From Cargo: https://github.com/rust-lang/cargo/blob/57622d793935a662b5f14ca728a2989c14833d37/src/cargo/util/toml/targets.rs#L127
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
        .get_or_insert_with(|| package_name.replace('-', "_"));

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

/// From Cargo: https://github.com/rust-lang/cargo/blob/57622d793935a662b5f14ca728a2989c14833d37/src/cargo/util/toml/targets.rs#L231
#[allow(clippy::too_many_arguments)]
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

/// From Cargo: https://github.com/rust-lang/cargo/blob/57622d793935a662b5f14ca728a2989c14833d37/src/cargo/util/toml/targets.rs#L333
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

/// From Cargo: https://github.com/rust-lang/cargo/blob/57622d793935a662b5f14ca728a2989c14833d37/src/cargo/util/toml/targets.rs#L355
pub fn resolve_examples(
    toml_examples: Option<&Vec<TomlExampleTarget>>,
    package_root: &Path,
    edition: Edition,
    autodiscover: Option<bool>,
    warnings: &mut Vec<String>,
    errors: &mut Vec<String>,
) -> CargoResult<Vec<TomlExampleTarget>> {
    let mut inferred = || infer_from_directory(package_root, Path::new(DEFAULT_EXAMPLE_DIR_NAME));

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

/// From Cargo: https://github.com/rust-lang/cargo/blob/57622d793935a662b5f14ca728a2989c14833d37/src/cargo/util/toml/targets.rs#L412
pub fn resolve_tests(
    toml_tests: Option<&Vec<TomlTestTarget>>,
    package_root: &Path,
    edition: Edition,
    autodiscover: Option<bool>,
    warnings: &mut Vec<String>,
    errors: &mut Vec<String>,
) -> CargoResult<Vec<TomlTestTarget>> {
    let mut inferred = || infer_from_directory(package_root, Path::new(DEFAULT_TEST_DIR_NAME));

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

/// From Cargo: https://github.com/rust-lang/cargo/blob/57622d793935a662b5f14ca728a2989c14833d37/src/cargo/util/toml/targets.rs#L462
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

    let mut inferred = || infer_from_directory(package_root, Path::new(DEFAULT_BENCH_DIR_NAME));

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

/// From Cargo: https://github.com/rust-lang/cargo/blob/57622d793935a662b5f14ca728a2989c14833d37/src/cargo/util/toml/targets.rs#L735
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

/// From Cargo: https://github.com/rust-lang/cargo/blob/57622d793935a662b5f14ca728a2989c14833d37/src/cargo/util/toml/targets.rs#L564
#[allow(clippy::too_many_arguments)]
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

/// From Cargo: https://github.com/rust-lang/cargo/blob/57622d793935a662b5f14ca728a2989c14833d37/src/cargo/util/toml/targets.rs#L529
#[allow(clippy::too_many_arguments)]
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

/// From Cargo: https://github.com/rust-lang/cargo/blob/57622d793935a662b5f14ca728a2989c14833d37/src/cargo/util/toml/targets.rs#L638
fn inferred_lib(package_root: &Path) -> Option<PathBuf> {
    let lib = Path::new("src").join("lib.rs");
    if package_root.join(&lib).exists() {
        Some(lib)
    } else {
        None
    }
}

/// From Cargo: https://github.com/rust-lang/cargo/blob/57622d793935a662b5f14ca728a2989c14833d37/src/cargo/util/toml/targets.rs#L647
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

/// From Cargo: https://github.com/rust-lang/cargo/blob/57622d793935a662b5f14ca728a2989c14833d37/src/cargo/util/toml/targets.rs#L660
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

/// From Cargo: https://github.com/rust-lang/cargo/blob/57622d793935a662b5f14ca728a2989c14833d37/src/cargo/util/toml/targets.rs#L674
fn infer_any(package_root: &Path, entry: &DirEntry) -> Option<(String, PathBuf)> {
    if entry.file_type().map_or(false, |t| t.is_dir()) {
        infer_subdirectory(package_root, entry)
    } else if entry.path().extension().and_then(|p| p.to_str()) == Some("rs") {
        infer_file(package_root, entry)
    } else {
        None
    }
}

/// From Cargo: https://github.com/rust-lang/cargo/blob/57622d793935a662b5f14ca728a2989c14833d37/src/cargo/util/toml/targets.rs#L684
fn infer_file(package_root: &Path, entry: &DirEntry) -> Option<(String, PathBuf)> {
    let path = entry.path();
    let stem = path.file_stem()?.to_str()?.to_owned();
    let path = path
        .strip_prefix(package_root)
        .map(|p| p.to_owned())
        .unwrap_or(path);
    Some((stem, path))
}

/// From Cargo: https://github.com/rust-lang/cargo/blob/57622d793935a662b5f14ca728a2989c14833d37/src/cargo/util/toml/targets.rs#L694
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

/// From Cargo: https://github.com/rust-lang/cargo/blob/57622d793935a662b5f14ca728a2989c14833d37/src/cargo/util/toml/targets.rs#L713
#[allow(clippy::too_many_arguments)]
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

/// From Cargo: https://github.com/rust-lang/cargo/blob/57622d793935a662b5f14ca728a2989c14833d37/src/cargo/util/toml/targets.rs#L809
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

/// From Cargo: https://github.com/rust-lang/cargo/blob/57622d793935a662b5f14ca728a2989c14833d37/src/cargo/util/toml/targets.rs#L873
///
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
        [target_path_file, target_path_subdir]
    }

    let target_name = name_or_panic(target);
    let commonly_wrong_paths = possible_target_paths(target_name, target_kind, true);
    let possible_paths = possible_target_paths(target_name, target_kind, false);
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

/// From Cargo: https://github.com/rust-lang/cargo/blob/57622d793935a662b5f14ca728a2989c14833d37/src/cargo/util/toml/targets.rs#L946
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

/// From Cargo: https://github.com/rust-lang/cargo/blob/57622d793935a662b5f14ca728a2989c14833d37/src/cargo/util/toml/targets.rs#L1003
///
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

/// From Cargo: https://github.com/rust-lang/cargo/blob/57622d793935a662b5f14ca728a2989c14833d37/src/cargo/util/toml/targets.rs#L1022
fn name_or_panic(target: &TomlTarget) -> &str {
    target
        .name
        .as_deref()
        .unwrap_or_else(|| panic!("target name is required"))
}

/// From Cargo: https://github.com/rust-lang/cargo/blob/57622d793935a662b5f14ca728a2989c14833d37/src/cargo/util/toml/targets.rs#L1029
fn validate_lib_name(target: &TomlTarget, warnings: &mut Vec<String>) -> CargoResult<()> {
    validate_target_name(target, "library", "lib", warnings)?;
    let name = name_or_panic(target);
    if name.contains('-') {
        anyhow::bail!("library target names cannot contain hyphens: {}", name)
    }

    Ok(())
}

/// From Cargo: https://github.com/rust-lang/cargo/blob/57622d793935a662b5f14ca728a2989c14833d37/src/cargo/util/toml/targets.rs#L1039
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

/// From Cargo: https://github.com/rust-lang/cargo/blob/57622d793935a662b5f14ca728a2989c14833d37/src/cargo/util/toml/targets.rs#L1052
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

/// From Cargo: https://github.com/rust-lang/cargo/blob/57622d793935a662b5f14ca728a2989c14833d37/src/cargo/util/toml/targets.rs#L1081
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

/// From Cargo: https://github.com/rust-lang/cargo/blob/57622d793935a662b5f14ca728a2989c14833d37/src/cargo/util/toml/targets.rs#L1100
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

/// From Cargo: https://github.com/rust-lang/cargo/blob/57622d793935a662b5f14ca728a2989c14833d37/src/cargo/util/toml/targets.rs#L1117
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

/// From Cargo: https://github.com/rust-lang/cargo/blob/57622d793935a662b5f14ca728a2989c14833d37/src/cargo/util/toml/targets.rs#L1139
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
