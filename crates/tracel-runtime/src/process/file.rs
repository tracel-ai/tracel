//! Writing the files the process shares with the program that launches it.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Writes `contents` to `path` atomically: to `<path>.tmp` first, then renamed to `path`, so
/// `path` is never seen half written. Nothing is left at `<path>.tmp` when it fails.
pub fn write_atomically(path: &Path, contents: &str) -> io::Result<()> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    let written = fs::write(&tmp, contents).and_then(|()| fs::rename(&tmp, path));
    if written.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    written
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::*;

    const CONTENTS: &str = "{}\n";

    fn entries(dir: &Path) -> Vec<OsString> {
        fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect()
    }

    #[test]
    fn the_file_holds_the_contents_written() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("jobs.json");

        write_atomically(&path, CONTENTS).unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap(), CONTENTS);
    }

    #[test]
    fn the_file_replaces_an_existing_one() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("jobs.json");
        fs::write(&path, "stale").unwrap();

        write_atomically(&path, CONTENTS).unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap(), CONTENTS);
    }

    #[test]
    fn a_written_file_leaves_no_temporary_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("jobs.json");

        write_atomically(&path, CONTENTS).unwrap();

        assert_eq!(entries(dir.path()), [path.file_name().unwrap()]);
    }

    #[test]
    fn a_failed_rename_leaves_no_temporary_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("jobs.json");
        fs::create_dir(&path).unwrap();
        fs::write(path.join("kept"), "").unwrap();

        write_atomically(&path, "{}").unwrap_err();

        assert_eq!(entries(dir.path()), [path.file_name().unwrap()]);
    }
}
