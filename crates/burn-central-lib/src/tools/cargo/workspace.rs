#![allow(dead_code)]

use std::path::{Path, PathBuf};

type CargoResult<T> = anyhow::Result<T>;

use anyhow::anyhow;

use crate::print_debug;
use crate::tools::cargo::paths;

use super::paths::normalize_path;

use super::toml::InheritableFields;

/// From Cargo: https://github.com/rust-lang/cargo/blob/57622d793935a662b5f14ca728a2989c14833d37/src/cargo/core/workspace.rs#L133
///
/// Configuration of a workspace in a manifest.
#[derive(Debug, Clone)]
pub enum WorkspaceConfig {
    /// Indicates that `[workspace]` was present and the members were
    /// optionally specified as well.
    Root(Box<WorkspaceRootConfig>),

    /// Indicates that `[workspace]` was present and the `root` field is the
    /// optional value of `package.workspace`, if present.
    Member { root: Option<String> },
}

/// From Cargo: https://github.com/rust-lang/cargo/blob/57622d793935a662b5f14ca728a2989c14833d37/src/cargo/core/workspace.rs#L143
///
/// Only modification is the debug print macro from cargo {debug!} to here {print_debug!}.
impl WorkspaceConfig {
    pub fn inheritable(&self) -> Option<&InheritableFields> {
        match self {
            WorkspaceConfig::Root(root) => Some(&root.inheritable_fields),
            WorkspaceConfig::Member { .. } => None,
        }
    }

    /// Returns the path of the workspace root based on this `[workspace]` configuration.
    ///
    /// Returns `None` if the root is not explicitly known.
    ///
    /// * `self_path` is the path of the manifest this `WorkspaceConfig` is located.
    /// * `look_from` is the path where discovery started (usually the current
    ///   working directory), used for `workspace.exclude` checking.
    fn get_ws_root(&self, self_path: &Path, look_from: &Path) -> Option<PathBuf> {
        match self {
            WorkspaceConfig::Root(ances_root_config) => {
                print_debug!("find_root - found a root checking exclusion");
                if !ances_root_config.is_excluded(look_from) {
                    print_debug!("find_root - found!");
                    Some(self_path.to_owned())
                } else {
                    None
                }
            }
            WorkspaceConfig::Member {
                root: Some(path_to_root),
            } => {
                print_debug!("find_root - found pointer");
                Some(read_root_pointer(self_path, path_to_root))
            }
            WorkspaceConfig::Member { .. } => None,
        }
    }
}

/// From Cargo: https://github.com/rust-lang/cargo/blob/57622d793935a662b5f14ca728a2989c14833d37/src/cargo/core/workspace.rs#L1938
///
/// The only modification is the debug print macro from cargo {debug!} to here {print_debug!}.
fn read_root_pointer(member_manifest: &Path, root_link: &str) -> PathBuf {
    let path = member_manifest
        .parent()
        .unwrap()
        .join(root_link)
        .join("Cargo.toml");
    print_debug!("find_root - pointer {}", path.display());
    paths::normalize_path(&path)
}

/// From Cargo: https://github.com/rust-lang/cargo/blob/57622d793935a662b5f14ca728a2989c14833d37/src/cargo/core/workspace.rs#L185
///
/// Intermediate configuration of a workspace root in a manifest.
///
/// Knows the Workspace Root path, as well as `members` and `exclude` lists of path patterns, which
/// together tell if some path is recognized as a member by this root or not.
#[derive(Debug, Clone)]
pub struct WorkspaceRootConfig {
    root_dir: PathBuf,
    members: Option<Vec<String>>,
    default_members: Option<Vec<String>>,
    exclude: Vec<String>,
    inheritable_fields: InheritableFields,
    custom_metadata: Option<toml::Value>,
}

/// From Cargo (with a few functions removed): https://github.com/rust-lang/cargo/blob/57622d793935a662b5f14ca728a2989c14833d37/src/cargo/core/workspace.rs#L1776
impl WorkspaceRootConfig {
    /// Creates a new Intermediate Workspace Root configuration.
    pub fn new(
        root_dir: &Path,
        members: &Option<Vec<String>>,
        default_members: &Option<Vec<String>>,
        exclude: &Option<Vec<String>>,
        inheritable: &Option<InheritableFields>,
        custom_metadata: &Option<toml::Value>,
    ) -> WorkspaceRootConfig {
        WorkspaceRootConfig {
            root_dir: root_dir.to_path_buf(),
            members: members.clone(),
            default_members: default_members.clone(),
            exclude: exclude.clone().unwrap_or_default(),
            inheritable_fields: inheritable.clone().unwrap_or_default(),
            custom_metadata: custom_metadata.clone(),
        }
    }
    /// Checks the path against the `excluded` list.
    ///
    /// This method does **not** consider the `members` list.
    fn is_excluded(&self, manifest_path: &Path) -> bool {
        let excluded = self
            .exclude
            .iter()
            .any(|ex| manifest_path.starts_with(self.root_dir.join(ex)));

        let explicit_member = match self.members {
            Some(ref members) => members
                .iter()
                .any(|mem| manifest_path.starts_with(self.root_dir.join(mem))),
            None => false,
        };

        !explicit_member && excluded
    }

    fn has_members_list(&self) -> bool {
        self.members.is_some()
    }

    pub fn inheritable(&self) -> &InheritableFields {
        &self.inheritable_fields
    }
}

/// From Cargo: https://github.com/rust-lang/cargo/blob/57622d793935a662b5f14ca728a2989c14833d37/src/cargo/core/workspace.rs#L1863
pub fn resolve_relative_path(
    label: &str,
    old_root: &Path,
    new_root: &Path,
    rel_path: &str,
) -> CargoResult<String> {
    let joined_path = normalize_path(&old_root.join(rel_path));
    match pathdiff::diff_paths(joined_path, new_root) {
        None => Err(anyhow!(
            "`{}` was defined in {} but could not be resolved with {}",
            label,
            old_root.display(),
            new_root.display()
        )),
        Some(path) => Ok(path
            .to_str()
            .ok_or_else(|| {
                anyhow!(
                    "`{}` resolved to non-UTF value (`{}`)",
                    label,
                    path.display()
                )
            })?
            .to_owned()),
    }
}
