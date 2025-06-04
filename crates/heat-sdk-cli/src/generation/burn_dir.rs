use std::{collections::HashMap, path::PathBuf};

use super::{crate_gen::GeneratedCrate, filetree::FileTree};
use std::path::Path;

pub struct BurnDir {
    crates: HashMap<String, FileTree>,
    binaries: HashMap<String, FileTree>,
    // config: ..,
    // data: ..,
    // logs: ..,
}

const BURN_DIR_NAME: &str = ".burn";
const BURN_BIN_DIR_NAME: &str = "bin";
const BURN_CRATES_DIR_NAME: &str = "crates";
const BURN_ARTIFACTS_DIR_NAME: &str = "artifacts";

impl BurnDir {
    pub fn init(&self, user_crate_dir: &Path) {
        std::fs::create_dir_all(user_crate_dir.join(BURN_DIR_NAME))
            .expect("Should be able to create heat dir.");
        std::fs::write(user_crate_dir.join(BURN_DIR_NAME).join(".gitignore"), "*")
            .expect("Should be able to write gitignore file.");
    }

    pub fn new() -> Self {
        BurnDir {
            crates: HashMap::new(),
            binaries: HashMap::new(),
        }
    }

    pub fn from_file_tree(file_tree: FileTree) -> anyhow::Result<Self> {
        let mut burn_dir_files = match file_tree {
            FileTree::Directory(_, dir_items) => dir_items,
            _ => return Err(anyhow::anyhow!("Heat directory is not a directory")),
        };

        let mut crates = HashMap::new();
        let mut binaries = HashMap::new();

        let bin_dir = burn_dir_files
            .iter()
            .position(
                |item| matches!(item, FileTree::Directory(name, _) if name == BURN_BIN_DIR_NAME),
            )
            .map(|index| burn_dir_files.swap_remove(index));

        if let Some(FileTree::Directory(_, mut bin_items)) = bin_dir {
            for item in bin_items.drain(..) {
                if let FileTree::FileRef(ref name) = item {
                    binaries.insert(name.clone(), item);
                }
            }
        }

        let crates_dir = burn_dir_files
            .iter()
            .position(
                |item| matches!(item, FileTree::Directory(name, _) if name == BURN_CRATES_DIR_NAME),
            )
            .map(|index| burn_dir_files.swap_remove(index));

        if let Some(FileTree::Directory(_, mut crate_items)) = crates_dir {
            for item in crate_items.drain(..) {
                if let FileTree::Directory(ref name, _) = item {
                    crates.insert(name.clone(), item);
                }
            }
        }

        Ok(BurnDir { crates, binaries })
    }

    pub fn try_from_path(user_dir_path: &Path) -> anyhow::Result<Self> {
        let file_tree = FileTree::read_from(
            &user_dir_path.join(BURN_DIR_NAME),
            &[std::env::consts::EXE_SUFFIX],
            &["target"],
        )?;

        BurnDir::from_file_tree(file_tree)
    }

    pub fn add_crate(&mut self, crate_name: &str, crate_gen: GeneratedCrate) {
        self.crates
            .insert(crate_name.to_string(), crate_gen.into_file_tree());
    }

    pub fn get_crate(&self, crate_name: &str) -> Option<&FileTree> {
        self.crates.get(crate_name)
    }

    pub fn remove_crate(&mut self, crate_name: &str) -> Option<FileTree> {
        self.crates.remove(crate_name)
    }

    pub fn add_binary(&mut self, binary_name: &str, binary: FileTree) {
        self.binaries.insert(binary_name.to_string(), binary);
    }

    pub fn get_binary_path(&self, bin_name: &str) -> Option<PathBuf> {
        match self.binaries.get(bin_name)? {
            FileTree::FileRef(name) => Some(PathBuf::from(format!(
                "{}/{}/{}",
                BURN_DIR_NAME,
                BURN_BIN_DIR_NAME,
                name.clone()
            ))),
            FileTree::File(name, _) => Some(PathBuf::from(format!(
                "{}/{}/{}",
                BURN_DIR_NAME,
                BURN_BIN_DIR_NAME,
                name.clone()
            ))),
            _ => None,
        }
    }

    pub fn get_crate_path(&self, user_crate_dir: &Path, crate_name: &str) -> Option<PathBuf> {
        match self.crates.get(crate_name)? {
            FileTree::Directory(name, _) => Some(
                user_crate_dir
                    .join(BURN_DIR_NAME)
                    .join(BURN_CRATES_DIR_NAME)
                    .join(name),
            ),
            _ => None,
        }
    }

    #[allow(dead_code)]
    pub fn get_crates_dir(&self, user_crate_dir: &Path) -> PathBuf {
        user_crate_dir
            .join(BURN_DIR_NAME)
            .join(BURN_CRATES_DIR_NAME)
    }

    pub fn get_bin_dir(&self, user_crate_dir: &Path) -> PathBuf {
        user_crate_dir.join(BURN_DIR_NAME).join(BURN_BIN_DIR_NAME)
    }

    pub fn get_artifacts_dir(&self, user_crate_dir: &Path) -> PathBuf {
        user_crate_dir
            .join(BURN_DIR_NAME)
            .join(BURN_ARTIFACTS_DIR_NAME)
    }

    pub fn get_crate_target_path(&self, crate_name: &str) -> Option<PathBuf> {
        let new_target_dir = std::env::var("BURN_TARGET_DIR").ok();

        match self.crates.get(crate_name)? {
            FileTree::Directory(name, _) => match new_target_dir {
                Some(target_dir) => Some(PathBuf::from(target_dir)),
                None => Some(PathBuf::from(format!(
                    "{}/{}/{}/target",
                    BURN_DIR_NAME,
                    BURN_CRATES_DIR_NAME,
                    name.clone()
                ))),
            },
            _ => None,
        }
    }

    pub fn write_bin_dir(&self, user_crate_dir: &Path) {
        let bin_dir_path = format!(
            "{}/{}/{}",
            user_crate_dir
                .to_str()
                .expect("User crate dir should be a valid path."),
            BURN_DIR_NAME,
            BURN_BIN_DIR_NAME
        );
        let bin_dir_path = Path::new(&bin_dir_path);
        std::fs::create_dir_all(bin_dir_path).expect("Should be able to create bin dir.");

        for item in self.binaries.values() {
            item.write_to(bin_dir_path)
                .expect("Should be able to write binary to file.");
        }
    }

    pub fn write_crates_dir(&self, user_crate_dir: &Path) {
        let crates_dir_path = format!(
            "{}/{}/{}",
            user_crate_dir
                .to_str()
                .expect("User crate dir should be a valid path."),
            BURN_DIR_NAME,
            BURN_CRATES_DIR_NAME
        );
        let crates_dir_path = Path::new(&crates_dir_path);
        std::fs::create_dir_all(crates_dir_path).expect("Should be able to create crates dir.");

        for item in self.crates.values() {
            item.write_to(crates_dir_path)
                .expect("Should be able to write crate to file.");
        }
    }

    #[allow(dead_code)]
    pub fn into_file_tree(self) -> FileTree {
        let mut burn_dir = vec![];

        let mut bin_dir = vec![];
        for (_name, item) in self.binaries {
            bin_dir.push(item);
        }
        burn_dir.push(FileTree::Directory(BURN_BIN_DIR_NAME.to_string(), bin_dir));

        let mut crates_dir = vec![];
        for (_name, item) in self.crates {
            crates_dir.push(item);
        }
        burn_dir.push(FileTree::Directory(
            BURN_CRATES_DIR_NAME.to_string(),
            crates_dir,
        ));

        FileTree::Directory(BURN_DIR_NAME.to_string(), burn_dir)
    }

    #[allow(dead_code)]
    pub fn write_all_to(self, path: &Path) -> Self {
        let file_tree = self.into_file_tree();
        file_tree
            .write_to(path)
            .expect("Should be able to write heat dir to file.");
        Self::from_file_tree(file_tree).expect("Should be able to read heat dir from file.")
    }
}
