use crate::{
    commands::{BuildCommand, RunCommand, RunParams},
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

    fn get_generated_crate_path(&self) -> PathBuf {
        let crate_name = self
            .generated_crate_name
            .as_ref()
            .expect("Generated crate name should exist.");
        self.heat_dir
            .get_crate_path(&self.user_crate_dir, crate_name)
            .expect("Crate path should exist.")
    }

    pub fn set_generated_crate_name(&mut self, name: String) {
        self.generated_crate_name = Some(name);
    }

    fn set_generated_crate(&mut self, generated_crate: GeneratedCrate) {
        let crate_name = generated_crate.name();
        if self.heat_dir.get_crate(&crate_name).is_some() {
            self.heat_dir.remove_crate(&crate_name);
        }
        self.heat_dir.add_crate(&crate_name, generated_crate);
    }

    fn get_target_exe_path(&self) -> Option<PathBuf> {
        let target_path = self
            .heat_dir
            .get_crate_target_path(self.generated_crate_name.as_ref()?)?;

        let full_path = self
            .user_crate_dir
            .join(target_path)
            .join(&self.build_profile)
            .join(format!(
                "{}{}",
                self.generated_crate_name
                    .as_ref()
                    .expect("Generated crate name should exist."),
                std::env::consts::EXE_SUFFIX
            ));
        print_info!(
            "target exe path: {}",
            full_path.to_str().expect("Path should be valid")
        );
        Some(full_path)
    }

    pub fn make_run_command(&self, cmd_desc: &RunCommand) -> std::process::Command {
        match &cmd_desc.run_params {
            RunParams::Training {
                function,
                config_path,
                project,
                key,
            } => {
                let bin_exe_path = self
                    .get_binary_exe_path(&cmd_desc.run_id)
                    .expect("Binary exe path should exist.");
                let mut command = std::process::Command::new(bin_exe_path);
                command
                    .current_dir(&self.user_crate_dir)
                    .env("HEAT_PROJECT_DIR", &self.user_crate_dir)
                    .args(["--project", project])
                    .args(["--key", key])
                    .args(["train", function, config_path]);
                command
            }
        }
    }

    pub fn generate_crate(&mut self, build_cmd_desc: &BuildCommand) -> anyhow::Result<()> {
        let generated_crate = crate::generation::crate_gen::create_crate(
            self.generated_crate_name
                .as_ref()
                .expect("Generated crate name should exist."),
            &self.user_project_name,
            &self.user_crate_dir.to_str().unwrap(),
            vec![&build_cmd_desc.backend.to_string()],
            &build_cmd_desc.backend,
        );

        self.set_generated_crate(generated_crate);
        self.heat_dir.write_crates_dir(&self.user_crate_dir);

        Ok(())
    }

    pub fn make_build_command(
        &self,
        cmd_desc: &BuildCommand,
    ) -> anyhow::Result<std::process::Command> {
        let profile_arg = match self.build_profile.as_str() {
            "release" => "--release",
            "debug" => "--debug",
            _ => {
                return Err(anyhow::anyhow!(format!(
                    "Invalid profile: {}",
                    self.build_profile
                )));
            }
        };

        let mut build_command = std::process::Command::new("cargo");
        build_command
            .arg("build")
            .arg(profile_arg)
            .arg("--no-default-features")
            .current_dir(&self.user_crate_dir)
            .env("HEAT_PROJECT_DIR", &self.user_crate_dir)
            .args([
                "--manifest-path",
                self.get_generated_crate_path()
                    .join("Cargo.toml")
                    .to_str()
                    .unwrap(),
            ])
            .args(["--message-format", "short"]);

        Ok(build_command)
    }

    fn get_binary_exe_path(&self, run_id: &str) -> Option<PathBuf> {
        let bin_name = self.bin_name_from_run_id(run_id);
        let binary_path = self.heat_dir.get_binary_path(&bin_name)?;
        let full_path = self.user_crate_dir.join(binary_path);
        print_info!("Binary exe path: {:?}", full_path);
        Some(full_path)
    }

    fn bin_name_from_run_id(&self, run_id: &str) -> String {
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
        let maybe_dest_exe_path = self.get_binary_exe_path(run_id);

        let target_bin_name = self.bin_name_from_run_id(run_id);
        let dest_exe_path = maybe_dest_exe_path.unwrap_or_else(|| {
            self.heat_dir
                .get_bin_dir(&self.user_crate_dir)
                .join(&target_bin_name)
        });

        self.heat_dir.write_bin_dir(&self.user_crate_dir);

        match std::fs::copy(src_exe_path, dest_exe_path) {
            Ok(_) => {
                self.heat_dir
                    .add_binary(&target_bin_name, FileTree::new_file_ref(&target_bin_name));
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
}
