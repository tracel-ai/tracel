//! File-backed bundle implementation for artifacts.
//!
//! This module provides an implementation of the `BundleSink` and `BundleSource` traits that uses the local filesystem to store artifact files.
//! It supports temporary bundles in the system temp directory or inside a parent of the caller's
//! choosing (which clean up after themselves), and persistent bundles rooted at a specified
//! directory.
//! The implementation ensures that file paths are sanitized to prevent directory traversal, and that concurrent writes to the same path are handled safely using temporary files and atomic renames.

use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use sha2::Digest;
use tempfile::TempDir;

use crate::bundle::{BundleSink, BundleSource};
use crate::tools::path::{safe_join, sanitize_rel_path};
use crate::upload::{MultipartUploadSource, UploadError};

/// File-backed bundle that can both read and write artifact files.
#[derive(Debug)]
pub struct FsBundle {
    root: PathBuf,
    files: Vec<FsBundleFile>,
    seen: HashSet<String>,
    _temp: Option<TempDir>,
}

impl FsBundle {
    /// Create a writable bundle rooted at the provided directory.
    pub fn create(root: impl Into<PathBuf>) -> Result<Self, std::io::Error> {
        let root = root.into();
        fs::create_dir_all(&root)?;
        Ok(Self {
            root,
            files: Vec::new(),
            seen: HashSet::new(),
            _temp: None,
        })
    }

    /// Create a temporary writable bundle that cleans up on drop.
    pub fn temp() -> Result<Self, std::io::Error> {
        let temp = TempDir::new()?;
        let root = temp.path().to_path_buf();
        Ok(Self {
            root,
            files: Vec::new(),
            seen: HashSet::new(),
            _temp: Some(temp),
        })
    }

    /// Create a temporary writable bundle inside `parent`, cleaned up on drop.
    ///
    /// For staging beside a destination: a file renamed from here into another child of `parent`
    /// stays on one filesystem.
    pub fn temp_in(parent: impl AsRef<Path>) -> Result<Self, std::io::Error> {
        let parent = parent.as_ref();
        fs::create_dir_all(parent)?;
        let temp = tempfile::Builder::new()
            .prefix(".staging-")
            .tempdir_in(parent)?;
        let root = temp.path().to_path_buf();
        Ok(Self {
            root,
            files: Vec::new(),
            seen: HashSet::new(),
            _temp: Some(temp),
        })
    }

    /// Moves every registered file into `directory` and returns a bundle rooted there.
    ///
    /// Existing files under registered paths are replaced; other files in `directory` are left
    /// alone. Renames replace atomically where the platform supports it. Crossing a mount point
    /// falls back to a copy through a temporary destination file.
    ///
    /// If moving one file fails, files already moved remain in `directory` and this bundle's
    /// temporary root is cleaned up on return.
    pub fn move_into(self, directory: impl AsRef<Path>) -> Result<Self, std::io::Error> {
        let directory = directory.as_ref();
        fs::create_dir_all(directory)?;
        if fs::canonicalize(directory)?.starts_with(fs::canonicalize(&self.root)?) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "a bundle cannot be moved inside its own root",
            ));
        }

        let paths = self.file_paths();
        for file in self.files() {
            let target = safe_join(directory, &file.rel_path).map_err(std::io::Error::other)?;
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            move_file(&file.abs_path, &target).map_err(|error| {
                std::io::Error::new(error.kind(), format!("{}: {error}", file.rel_path))
            })?;
        }

        Self::with_files(directory.to_path_buf(), paths).map_err(std::io::Error::other)
    }

    /// Create a read-oriented bundle backed by an existing root + file list.
    pub fn with_files(root: PathBuf, files: Vec<String>) -> Result<Self, String> {
        let mut bundle = Self {
            root,
            files: Vec::new(),
            seen: HashSet::new(),
            _temp: None,
        };
        for path in files {
            bundle.register_file(&path)?;
        }
        Ok(bundle)
    }

    /// Root directory for the bundle files.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Files written into the bundle.
    pub fn files(&self) -> &[FsBundleFile] {
        &self.files
    }

    /// Relative file paths currently indexed by this bundle.
    pub fn file_paths(&self) -> Vec<String> {
        self.files.iter().map(|f| f.rel_path.clone()).collect()
    }

    /// Register an existing file path in the bundle, ensuring it is valid and not duplicated.
    fn register_file(&mut self, path: &str) -> Result<(), String> {
        let rel = sanitize_rel_path(path)?.to_string_lossy().to_string();
        if rel.is_empty() || !self.seen.insert(rel.clone()) {
            return Err(format!("Duplicate bundle path: {rel}"));
        }

        self.files.push(FsBundleFile {
            rel_path: rel.clone(),
            abs_path: self.root.join(&rel),
            size_bytes: None,
            checksum: None,
        });

        Ok(())
    }

    /// Delete all files in this bundle from the filesystem. This is idempotent and can be used for cleanup.
    pub fn delete(self) -> Result<(), std::io::Error> {
        for file in &self.files {
            let path = safe_join(&self.root, &file.rel_path);
            if let Ok(path) = path {
                match fs::remove_file(path) {
                    Ok(()) => {}
                    Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                    Err(err) => return Err(err),
                }
            }
        }
        Ok(())
    }

    fn clear_temp_files(&self) {
        for file in &self.files {
            let tmp = temp_path(&file.abs_path);
            if let Ok(tmp) = tmp {
                if tmp.exists() {
                    let _ = fs::remove_file(tmp);
                }
            }
        }
    }
}

impl Drop for FsBundle {
    fn drop(&mut self) {
        self.clear_temp_files();
    }
}

impl BundleSink for FsBundle {
    fn put_file<R: Read>(&mut self, path: &str, reader: &mut R) -> Result<(), String> {
        let rel = sanitize_rel_path(path).map_err(|e| e.to_string())?;
        let rel = rel.to_string_lossy().to_string();

        if !self.seen.insert(rel.clone()) {
            return Err(format!("Duplicate bundle path: {rel}"));
        }

        let dest = safe_join(&self.root, &rel).map_err(|e| e.to_string())?;
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }

        let tmp = temp_path(&dest).map_err(|e| e.to_string())?;
        let mut file = match File::create(&tmp) {
            Ok(file) => file,
            Err(e) => {
                self.seen.remove(&rel);
                return Err(e.to_string());
            }
        };

        let mut hasher = sha2::Sha256::new();
        let mut buf = [0u8; 1024 * 64];
        let mut total = 0u64;

        loop {
            let read = match reader.read(&mut buf) {
                Ok(read) => read,
                Err(e) => {
                    let _ = fs::remove_file(&tmp);
                    self.seen.remove(&rel);
                    return Err(e.to_string());
                }
            };
            if read == 0 {
                break;
            }
            if let Err(e) = file.write_all(&buf[..read]) {
                let _ = fs::remove_file(&tmp);
                self.seen.remove(&rel);
                return Err(e.to_string());
            }
            hasher.update(&buf[..read]);
            total += read as u64;
        }

        let checksum = format!("{:x}", hasher.finalize());

        if let Err(err) = finalize_temp_file(&tmp, &dest) {
            self.seen.remove(&rel);
            let _ = fs::remove_file(&tmp);
            return Err(err.to_string());
        }

        self.files.push(FsBundleFile {
            rel_path: rel,
            abs_path: dest,
            size_bytes: Some(total),
            checksum: Some(checksum),
        });

        Ok(())
    }
}

/// File descriptor emitted by a file-backed bundle.
#[derive(Debug, Clone)]
pub struct FsBundleFile {
    /// Relative path within the bundle.
    pub rel_path: String,
    /// Absolute file system path for the cached file.
    pub abs_path: PathBuf,
    /// Size in bytes, when known.
    pub size_bytes: Option<u64>,
    /// SHA-256 checksum (hex), when known.
    pub checksum: Option<String>,
}

impl BundleSource for FsBundle {
    fn open(&self, path: &str) -> Result<Box<dyn Read + Send>, String> {
        let rel = sanitize_rel_path(path).map_err(|e| e.to_string())?;
        let rel = rel.to_string_lossy().to_string();

        if !self.seen.contains(&rel) {
            return Err(format!("Bundle path not found: {rel}"));
        }

        let file_path = safe_join(&self.root, &rel).map_err(|e| e.to_string())?;
        let file = File::open(&file_path).map_err(|e| e.to_string())?;
        Ok(Box::new(file))
    }

    fn list(&self) -> Result<Vec<String>, String> {
        Ok(self.file_paths())
    }
}

fn temp_path(dest: &Path) -> Result<PathBuf, std::io::Error> {
    let file_name = dest
        .file_name()
        .ok_or_else(|| std::io::Error::other("Missing file name"))?
        .to_string_lossy();
    Ok(dest.with_file_name(format!(".{file_name}.partial")))
}

fn finalize_temp_file(tmp: &Path, dest: &Path) -> Result<(), std::io::Error> {
    // Unix rename replaces an existing file atomically. Windows requires the destination to be
    // removed first.
    #[cfg(windows)]
    if dest.exists() {
        fs::remove_file(dest)?;
    }

    fs::rename(tmp, dest)
}

fn move_file(from: &Path, to: &Path) -> Result<(), std::io::Error> {
    #[cfg(windows)]
    if to.exists() {
        fs::remove_file(to)?;
    }

    match fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::CrossesDevices => {
            copy_across_devices(from, to)
        }
        Err(error) => Err(error),
    }
}

fn copy_across_devices(from: &Path, to: &Path) -> Result<(), std::io::Error> {
    let tmp = temp_path(to)?;
    if tmp.exists() {
        fs::remove_file(&tmp)?;
    }
    if let Err(error) = fs::copy(from, &tmp) {
        let _ = fs::remove_file(&tmp);
        return Err(error);
    }
    if let Err(error) = finalize_temp_file(&tmp, to) {
        let _ = fs::remove_file(&tmp);
        return Err(error);
    }
    fs::remove_file(from)
}

impl MultipartUploadSource for FsBundle {
    fn file_len(&self, rel_path: &str) -> Result<u64, UploadError> {
        let source = safe_join(self.root(), rel_path).map_err(UploadError::InvalidMultipart)?;
        let metadata = std::fs::metadata(&source).map_err(|e| {
            UploadError::InvalidMultipart(format!(
                "Missing file for multipart upload {}: {}",
                rel_path, e
            ))
        })?;
        if !metadata.is_file() {
            return Err(UploadError::InvalidMultipart(format!(
                "Multipart upload source is not a file: {}",
                rel_path
            )));
        }

        Ok(metadata.len())
    }

    fn open_part(
        &self,
        rel_path: &str,
        offset: u64,
        size: u64,
    ) -> Result<Box<dyn Read + Send>, UploadError> {
        let source = safe_join(self.root(), rel_path).map_err(UploadError::InvalidMultipart)?;
        let mut file = File::open(&source).map_err(|e| UploadError::MultipartReader {
            rel_path: rel_path.to_string(),
            source: Box::new(e),
        })?;
        file.seek(SeekFrom::Start(offset))
            .map_err(|e| UploadError::MultipartReader {
                rel_path: rel_path.to_string(),
                source: Box::new(e),
            })?;
        Ok(Box::new(file.take(size)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_temp_bundle_in_a_parent_lives_there_and_leaves_on_drop() {
        let home = TempDir::new().unwrap();
        let parent = home.path().join("parent");
        let bundle = FsBundle::temp_in(&parent).unwrap();
        let root = bundle.root().to_path_buf();

        assert_eq!(root.parent(), Some(parent.as_path()));
        assert!(root.exists());

        drop(bundle);

        assert_eq!(parent.read_dir().unwrap().count(), 0);
    }

    #[test]
    fn moving_a_bundle_replaces_its_files_and_keeps_unregistered_files() {
        let home = TempDir::new().unwrap();
        let directory = home.path().join("landed");
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("weights.bin"), b"old").unwrap();
        fs::write(directory.join("notes.txt"), b"keep").unwrap();
        let mut bundle = FsBundle::temp_in(home.path()).unwrap();
        let staging = bundle.root().to_path_buf();
        bundle.put_file("weights.bin", &mut &b"new"[..]).unwrap();
        bundle
            .put_file("config/model.json", &mut &b"configuration"[..])
            .unwrap();

        let moved = bundle.move_into(&directory).unwrap();

        assert!(!staging.exists());
        assert_eq!(fs::read(directory.join("weights.bin")).unwrap(), b"new");
        assert_eq!(fs::read(directory.join("notes.txt")).unwrap(), b"keep");
        assert_eq!(
            fs::read(directory.join("config/model.json")).unwrap(),
            b"configuration"
        );
        assert_eq!(moved.file_paths(), vec!["weights.bin", "config/model.json"]);
    }

    #[test]
    fn a_bundle_cannot_be_moved_inside_its_own_root() {
        let mut bundle = FsBundle::temp().unwrap();
        bundle.put_file("weights.bin", &mut &b"model"[..]).unwrap();
        let directory = bundle.root().join("nested");

        let error = bundle.move_into(directory).unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    }
}
