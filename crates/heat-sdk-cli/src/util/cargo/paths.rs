use anyhow::{Context, Result};
use std::fs;
use std::path::{Component, Path, PathBuf};

/// From Cargo: https://github.com/rust-lang/cargo/blob/57622d793935a662b5f14ca728a2989c14833d37/crates/cargo-util/src/paths.rs#L160
///
/// Reads a file to a string.
///
/// Equivalent to [`std::fs::read_to_string`] with better error messages.
pub fn read(path: &Path) -> Result<String> {
    match String::from_utf8(read_bytes(path)?) {
        Ok(s) => Ok(s),
        Err(_) => anyhow::bail!("path at `{}` was not valid utf-8", path.display()),
    }
}

/// From Cargo: https://github.com/rust-lang/cargo/blob/57622d793935a662b5f14ca728a2989c14833d37/crates/cargo-util/src/paths.rs#L170
///
/// Reads a file into a bytes vector.
///
/// Equivalent to [`std::fs::read`] with better error messages.
pub fn read_bytes(path: &Path) -> Result<Vec<u8>> {
    fs::read(path).with_context(|| format!("failed to read `{}`", path.display()))
}

/// From Cargo: https://github.com/rust-lang/cargo/blob/57622d793935a662b5f14ca728a2989c14833d37/crates/cargo-util/src/paths.rs#L84
///
/// Normalize a path, removing things like `.` and `..`.
///
/// CAUTION: This does not resolve symlinks (unlike
/// [`std::fs::canonicalize`]). This may cause incorrect or surprising
/// behavior at times. This should be used carefully. Unfortunately,
/// [`std::fs::canonicalize`] can be hard to use correctly, since it can often
/// fail, or on Windows returns annoying device paths. This is a problem Cargo
/// needs to improve on.
pub fn normalize_path(path: &Path) -> PathBuf {
    let mut components = path.components().peekable();
    let mut ret = if let Some(c @ Component::Prefix(..)) = components.peek().cloned() {
        components.next();
        PathBuf::from(c.as_os_str())
    } else {
        PathBuf::new()
    };

    for component in components {
        match component {
            Component::Prefix(..) => unreachable!(),
            Component::RootDir => {
                ret.push(component.as_os_str());
            }
            Component::CurDir => {}
            Component::ParentDir => {
                ret.pop();
            }
            Component::Normal(c) => {
                ret.push(c);
            }
        }
    }
    ret
}

/// From Cargo: https://github.com/rust-lang/cargo/blob/57622d793935a662b5f14ca728a2989c14833d37/crates/cargo-util/src/paths.rs#L375
///
/// Converts a path to UTF-8 bytes.
pub fn path2bytes(path: &Path) -> Result<&[u8]> {
    #[cfg(unix)]
    {
        use std::os::unix::prelude::*;
        Ok(path.as_os_str().as_bytes())
    }
    #[cfg(windows)]
    {
        match path.as_os_str().to_str() {
            Some(s) => Ok(s.as_bytes()),
            None => Err(anyhow::format_err!(
                "invalid non-unicode path: {}",
                path.display()
            )),
        }
    }
}

/// From Cargo: https://github.com/rust-lang/cargo/blob/57622d793935a662b5f14ca728a2989c14833d37/src/cargo/util/toml/mod.rs#L2950
pub fn normalize_path_sep(path: PathBuf, context: &str) -> anyhow::Result<PathBuf> {
    let path = path
        .into_os_string()
        .into_string()
        .map_err(|_err| anyhow::format_err!("non-UTF8 path for {context}"))?;
    let path = normalize_path_string_sep(path);
    Ok(path.into())
}

/// From Cargo: https://github.com/rust-lang/cargo/blob/57622d793935a662b5f14ca728a2989c14833d37/src/cargo/util/toml/mod.rs#L2959
pub fn normalize_path_string_sep(path: String) -> String {
    if std::path::MAIN_SEPARATOR != '/' {
        path.replace(std::path::MAIN_SEPARATOR, "/")
    } else {
        path
    }
}
