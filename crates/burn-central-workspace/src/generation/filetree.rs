use std::{
    fs::OpenOptions,
    io::{Read, Write},
    path::Path,
};

use serde::{Deserialize, Serialize};

/// A tree structure representing a file system.
/// Can be written to and read from the file system.
#[derive(Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
pub enum FileTree {
    /// A file with a name and content.
    File(String, Vec<u8>),
    /// A reference to a file. Does not contain content.
    FileRef(String),
    /// A directory with a name and children.
    Directory(String, Vec<FileTree>),
}

impl FileTree {
    pub fn new_file(name: impl Into<String>, content: impl Into<Vec<u8>>) -> Self {
        FileTree::File(name.into(), content.into())
    }

    #[allow(dead_code)]
    pub fn new_file_ref(name: impl Into<String>) -> Self {
        FileTree::FileRef(name.into())
    }

    pub fn new_dir(name: impl Into<String>, children: impl Into<Vec<FileTree>>) -> Self {
        FileTree::Directory(name.into(), children.into())
    }

    pub fn try_insert(&mut self, file_tree: FileTree) -> Result<&mut Self, ()> {
        match self {
            FileTree::Directory(_, children) => {
                children.push(file_tree);
                Ok(self)
            }
            _ => Err(()),
        }
    }

    pub fn insert(&mut self, file_tree: FileTree) -> &mut Self {
        match self.try_insert(file_tree) {
            Ok(_) => self,
            Err(_) => panic!("Cannot insert into a file"),
        }
    }

    #[allow(dead_code)]
    pub fn read_from(
        path: &Path,
        ref_suffixes: &[&str],
        ignore_names: &[&str],
    ) -> std::io::Result<Self> {
        if path.is_file() {
            let mut buf = Vec::new();
            std::fs::File::open(path)?.read_to_end(&mut buf)?;
            if ref_suffixes
                .iter()
                .any(|&s| path.file_name().unwrap().to_string_lossy().ends_with(s))
            {
                return Ok(FileTree::FileRef(
                    path.file_name().unwrap().to_string_lossy().into_owned(),
                ));
            }
            Ok(FileTree::File(
                path.file_name().unwrap().to_string_lossy().into_owned(),
                buf,
            ))
        } else {
            let mut children = Vec::new();
            for entry in std::fs::read_dir(path)? {
                let entry = entry?;
                let name = entry.file_name();
                if ignore_names.iter().any(|&n| name == n) {
                    continue;
                }
                let file_tree = FileTree::read_from(&entry.path(), ref_suffixes, ignore_names)?;
                children.push(file_tree);
            }
            Ok(FileTree::Directory(
                path.file_name().unwrap().to_string_lossy().into_owned(),
                children,
            ))
        }
    }

    pub fn write_to(&self, path: &Path) -> std::io::Result<()> {
        match self {
            FileTree::File(name, content) => {
                let should_write =
                    if let Ok(mut file) = OpenOptions::new().read(true).open(path.join(name)) {
                        let mut buf = Vec::new();
                        file.read_to_end(&mut buf)?;
                        buf != *content
                    } else {
                        true
                    };
                if should_write {
                    let mut file = std::fs::File::create(path.join(name))?;
                    file.write_all(content)?;
                }
            }
            FileTree::FileRef(name) => {
                if !path.join(name).exists() {
                    std::fs::File::create(path.join(name))?;
                }
            }
            FileTree::Directory(name, children) => {
                let dir = path.join(name);
                std::fs::create_dir_all(&dir)?;
                for child in children {
                    child.write_to(&dir)?;
                }
            }
        }
        Ok(())
    }

    #[allow(dead_code)]
    pub fn get_name(&self) -> String {
        match self {
            FileTree::File(name, _) => name,
            FileTree::FileRef(name) => name,
            FileTree::Directory(name, _) => name,
        }
        .clone()
    }
}
