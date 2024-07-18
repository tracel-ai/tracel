use std::{
    fs::OpenOptions,
    io::{Read, Write},
    path::Path,
};

use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
pub enum FileTree {
    File(String, Vec<u8>),
    Directory(String, Vec<FileTree>),
}

impl FileTree {
    pub fn new_file(name: impl Into<String>, content: impl Into<Vec<u8>>) -> Self {
        FileTree::File(name.into(), content.into())
    }

    pub fn new_dir(name: impl Into<String>, children: impl Into<Vec<FileTree>>) -> Self {
        FileTree::Directory(name.into(), children.into())
    }

    pub fn as_dir(&self) -> Option<&Vec<FileTree>> {
        match self {
            FileTree::Directory(_, children) => Some(children),
            _ => None,
        }
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

    pub fn insert(&mut self, file_tree: FileTree) {
        match self.try_insert(file_tree) {
            Ok(_) => {}
            Err(_) => panic!("Cannot insert into a file"),
        }
    }

    pub fn read_from(path: &Path) -> std::io::Result<Self> {
        if path.is_file() {
            let mut buf = Vec::new();
            std::fs::File::open(path)?.read_to_end(&mut buf)?;
            Ok(FileTree::File(
                path.file_name().unwrap().to_string_lossy().to_string(),
                buf,
            ))
        } else {
            let mut children = Vec::new();
            for entry in std::fs::read_dir(path)? {
                let entry = entry?;
                children.push(FileTree::read_from(&entry.path())?);
            }
            Ok(FileTree::Directory(
                path.file_name().unwrap().to_string_lossy().to_string(),
                children,
            ))
        }
    }

    pub fn write_to(&self, path: &Path) -> std::io::Result<()> {
        match self {
            FileTree::File(name, content) => {
                // read file if it exists and check if it's the same
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
}
