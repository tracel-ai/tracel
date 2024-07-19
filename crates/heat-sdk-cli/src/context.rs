use crate::{
    generation::{FileTree, GeneratedCrate, HeatDir},
    print_info,
};
use std::path::PathBuf;

pub struct HeatCliContext {
    user_project_name: String,
    user_crate_dir: PathBuf,
    generated_crate_name: Option<String>,
    build_profile: String,
    heat_dir: HeatDir,
}

impl HeatCliContext {
    pub fn new(user_project_name: String, user_crate_dir: PathBuf) -> Self {
        let heat_dir = match HeatDir::try_from_path(&user_crate_dir) {
            Ok(heat_dir) => heat_dir,
            Err(_) => HeatDir::new(),
        };

        Self {
            user_project_name,
            user_crate_dir,
            generated_crate_name: None,
            build_profile: "release".to_string(),
            heat_dir,
        }
    }

    pub fn init(self) -> Self {
        self.heat_dir.init(&self.user_crate_dir);
        self
    }

    pub fn generated_crate_name(&self) -> Option<&String> {
        self.generated_crate_name.as_ref()
    }

    pub fn user_project_name(&self) -> &str {
        &self.user_project_name
    }

    pub fn user_crate_dir(&self) -> &PathBuf {
        &self.user_crate_dir
    }

    pub fn add_binary_ref(&mut self, name: &str) {
        self.heat_dir.add_binary(name, FileTree::new_file_ref(name));
    }

    pub fn get_binary_ref(&self, name: &str) -> Option<&FileTree> {
        self.heat_dir.get_binary(name)
    }

    pub fn get_binary_path(&self, name: &str) -> Option<String> {
        self.heat_dir.get_binary_path(name)
    }

    pub fn get_generated_crate_path(&self) -> String {
        let crate_name = self
            .generated_crate_name
            .as_ref()
            .expect("Generated crate name should exist.");
        self.heat_dir
            .get_crate_path(self.user_crate_dir(), crate_name)
            .expect("Crate path should exist.")
    }

    pub fn set_generated_crate_name(&mut self, name: String) {
        self.generated_crate_name = Some(name);
    }

    pub fn set_generated_crate(&mut self, generated_crate: GeneratedCrate) {
        let crate_name = generated_crate.name();
        if self.heat_dir.get_crate(&crate_name).is_some() {
            self.heat_dir.remove_crate(&crate_name);
        }
        self.heat_dir.add_crate(&crate_name, generated_crate);
    }

    pub fn get_target_exe_path(&self) -> Option<String> {
        let target_path = self
            .heat_dir
            .get_crate_target_path(self.generated_crate_name.as_ref()?)?;
        print_info!(
            "target exe path: {}",
            format!(
                "{}/{}/{}/{}{}",
                self.user_crate_dir.to_str().expect("Path should be valid"),
                target_path,
                self.build_profile,
                self.generated_crate_name
                    .as_ref()
                    .expect("Generated crate name should exist."),
                std::env::consts::EXE_SUFFIX
            )
        );
        Some(format!(
            "{}/{}/{}/{}{}",
            self.user_crate_dir.to_str().expect("Path should be valid"),
            target_path,
            self.build_profile,
            self.generated_crate_name
                .as_ref()
                .expect("Generated crate name should exist."),
            std::env::consts::EXE_SUFFIX
        ))
    }

    pub fn get_binary_exe_path(&self, run_id: &str) -> String {
        let binary_path = self.heat_dir.get_binary_dir_path();
        print_info!(
            "binary path: {}",
            format!(
                "{}/{}/{}",
                self.user_crate_dir.to_str().expect("Path should be valid"),
                binary_path,
                self.get_binary_exe_name(run_id)
            )
        );
        format!(
            "{}/{}/{}",
            self.user_crate_dir.to_str().expect("Path should be valid"),
            binary_path,
            self.get_binary_exe_name(run_id)
        )
    }

    pub fn get_binary_exe_name(&self, run_id: &str) -> String {
        format!(
            "{}-{}{}",
            self.generated_crate_name
                .as_ref()
                .expect("Generated crate name should exist."),
            run_id,
            std::env::consts::EXE_SUFFIX
        )
    }

    pub fn copy_executable_to_bin(&mut self, run_id: &str) -> anyhow::Result<()> {
        let src_exe_path = self
            .get_target_exe_path()
            .expect("Target exe path should exist.");
        let dest_exe_path = self.get_binary_exe_path(run_id);

        self.heat_dir.write_bin_dir(self.user_crate_dir());

        match std::fs::copy(src_exe_path, dest_exe_path) {
            Ok(_) => {
                self.add_binary_ref(&self.get_binary_exe_name(run_id));
            }
            Err(e) => {
                return Err(anyhow::anyhow!(format!(
                    "Failed to copy executable: {:?}",
                    e
                )));
            }
        }

        Ok(())
    }

    pub fn generate_crate(&mut self, generated_crate: GeneratedCrate) {
        self.set_generated_crate(generated_crate);
        self.heat_dir.write_crates_dir(self.user_crate_dir());
    }
}
