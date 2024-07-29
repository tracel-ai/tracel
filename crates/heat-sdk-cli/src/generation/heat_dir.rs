use std::{collections::HashMap, path::PathBuf};

use super::{crate_gen::GeneratedCrate, filetree::FileTree};
use std::path::Path;

pub struct HeatDir {
    crates: HashMap<String, FileTree>,
    binaries: HashMap<String, FileTree>,
    // config: ..,
    // data: ..,
    // logs: ..,
}

const HEAT_DIR_NAME: &str = ".heat";
const HEAT_BIN_DIR_NAME: &str = "bin";
const HEAT_CRATES_DIR_NAME: &str = "crates";
const HEAT_ARTIFACTS_DIR_NAME: &str = "artifacts";

impl HeatDir {
    pub fn init(&self, user_crate_dir: &PathBuf) {
        std::fs::create_dir_all(user_crate_dir.join(HEAT_DIR_NAME))
            .expect("Should be able to create heat dir.");
        std::fs::write(user_crate_dir.join(HEAT_DIR_NAME).join(".gitignore"), "*")
            .expect("Should be able to write gitignore file.");
    }

    pub fn new() -> Self {
        HeatDir {
            crates: HashMap::new(),
            binaries: HashMap::new(),
        }
    }

    pub fn from_file_tree(file_tree: FileTree) -> anyhow::Result<Self> {
        let mut heat_files = match file_tree {
            FileTree::Directory(_, dir_items) => dir_items,
            _ => return Err(anyhow::anyhow!("Heat directory is not a directory")),
        };

        let mut crates = HashMap::new();
        let mut binaries = HashMap::new();

        let bin_dir = heat_files
            .iter()
            .position(
                |item| matches!(item, FileTree::Directory(name, _) if name == HEAT_BIN_DIR_NAME),
            )
            .map(|index| heat_files.swap_remove(index));

        if let Some(FileTree::Directory(_, mut bin_items)) = bin_dir {
            for item in bin_items.drain(..) {
                if let FileTree::FileRef(ref name) = item {
                    binaries.insert(name.clone(), item);
                }
            }
        }

        let crates_dir = heat_files
            .iter()
            .position(
                |item| matches!(item, FileTree::Directory(name, _) if name == HEAT_CRATES_DIR_NAME),
            )
            .map(|index| heat_files.swap_remove(index));

        if let Some(FileTree::Directory(_, mut crate_items)) = crates_dir {
            for item in crate_items.drain(..) {
                if let FileTree::Directory(ref name, _) = item {
                    crates.insert(name.clone(), item);
                }
            }
        }

        Ok(HeatDir { crates, binaries })
    }

    pub fn try_from_path(user_dir_path: &Path) -> anyhow::Result<Self> {
        let file_tree = FileTree::read_from(
            &user_dir_path.join(HEAT_DIR_NAME),
            &[std::env::consts::EXE_SUFFIX],
            &["target"],
        )?;

        HeatDir::from_file_tree(file_tree)
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
                HEAT_DIR_NAME,
                HEAT_BIN_DIR_NAME,
                name.clone()
            ))),
            FileTree::File(name, _) => Some(PathBuf::from(format!(
                "{}/{}/{}",
                HEAT_DIR_NAME,
                HEAT_BIN_DIR_NAME,
                name.clone()
            ))),
            _ => None,
        }
    }

    pub fn get_crate_path(&self, user_crate_dir: &Path, crate_name: &str) -> Option<PathBuf> {
        match self.crates.get(crate_name)? {
            FileTree::Directory(name, _) => Some(
                user_crate_dir
                    .join(HEAT_DIR_NAME)
                    .join(HEAT_CRATES_DIR_NAME)
                    .join(name),
            ),
            _ => None,
        }
    }

    #[allow(dead_code)]
    pub fn get_crates_dir(&self, user_crate_dir: &Path) -> PathBuf {
        user_crate_dir
            .join(HEAT_DIR_NAME)
            .join(HEAT_CRATES_DIR_NAME)
    }

    pub fn get_bin_dir(&self, user_crate_dir: &Path) -> PathBuf {
        user_crate_dir.join(HEAT_DIR_NAME).join(HEAT_BIN_DIR_NAME)
    }

    pub fn get_artifacts_dir(&self, user_crate_dir: &Path) -> PathBuf {
        user_crate_dir
            .join(HEAT_DIR_NAME)
            .join(HEAT_ARTIFACTS_DIR_NAME)
    }

    pub fn get_crate_target_path(&self, crate_name: &str) -> Option<PathBuf> {
        match self.crates.get(crate_name)? {
            FileTree::Directory(name, _) => Some(PathBuf::from(format!(
                "{}/{}/{}/target",
                HEAT_DIR_NAME,
                HEAT_CRATES_DIR_NAME,
                name.clone()
            ))),
            _ => None,
        }
    }

    pub fn write_bin_dir(&self, user_crate_dir: &Path) {
        let bin_dir_path = format!(
            "{}/{}/{}",
            user_crate_dir
                .to_str()
                .expect("User crate dir should be a valid path."),
            HEAT_DIR_NAME,
            HEAT_BIN_DIR_NAME
        );
        let bin_dir_path = Path::new(&bin_dir_path);
        std::fs::create_dir_all(bin_dir_path).expect("Should be able to create bin dir.");

        for (_name, item) in &self.binaries {
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
            HEAT_DIR_NAME,
            HEAT_CRATES_DIR_NAME
        );
        let crates_dir_path = Path::new(&crates_dir_path);
        std::fs::create_dir_all(crates_dir_path).expect("Should be able to create crates dir.");

        for (_name, item) in &self.crates {
            item.write_to(crates_dir_path)
                .expect("Should be able to write crate to file.");
        }
    }

    #[allow(dead_code)]
    pub fn into_file_tree(self) -> FileTree {
        let mut heat_dir = vec![];

        let mut bin_dir = vec![];
        for (_name, item) in self.binaries {
            bin_dir.push(item);
        }
        heat_dir.push(FileTree::Directory(HEAT_BIN_DIR_NAME.to_string(), bin_dir));

        let mut crates_dir = vec![];
        for (_name, item) in self.crates {
            crates_dir.push(item);
        }
        heat_dir.push(FileTree::Directory(
            HEAT_CRATES_DIR_NAME.to_string(),
            crates_dir,
        ));

        FileTree::Directory(HEAT_DIR_NAME.to_string(), heat_dir)
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
