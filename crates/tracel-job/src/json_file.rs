//! Writing and reading the JSON files a program shares with the program that launches it.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde::de::DeserializeOwned;

/// Writes `value` to `path` as pretty-printed JSON ending with a newline: to `<path>.tmp` first,
/// then renamed to `path`, so `path` is never seen half written. Nothing is left at `<path>.tmp`
/// when it fails.
pub fn write<T: Serialize>(path: &Path, value: &T) -> io::Result<()> {
    let mut contents = serde_json::to_vec_pretty(value)?;
    contents.push(b'\n');

    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    let written = fs::write(&tmp, contents).and_then(|()| fs::rename(&tmp, path));
    if written.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    written
}

/// Reads the JSON document in the file at `path` as a `T`.
pub fn read<T: DeserializeOwned>(path: &Path) -> io::Result<T> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}
