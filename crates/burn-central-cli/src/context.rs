use crate::{
    commands::{BuildCommand, RunCommand, RunParams},
    config::Config,
    generation::{BurnDir, FileTree, GeneratedCrate},
    print_info,
};
use std::path::PathBuf;

pub struct BurnCentralCliContext {
    user_project_name: String,
    user_crate_dir: PathBuf,
    generated_crate_name: Option<String>,
    build_profile: String,
    burn_dir: BurnDir,
    api_endpoint: url::Url,
    wss: bool,
}

impl BurnCentralCliContext {
    pub fn new(config: &Config) -> Self {
        let user_project_name = std::env::var("CARGO_PKG_NAME").expect("CARGO_PKG_NAME not set");
        let user_crate_dir: PathBuf = std::env::var("CARGO_MANIFEST_DIR")
            .expect("CARGO_MANIFEST_DIR not set")
            .into();

        let burn_dir = BurnDir::try_from_path(&user_crate_dir).unwrap_or_else(|_| BurnDir::new());

        Self {
            user_project_name,
            user_crate_dir,
            generated_crate_name: None,
            build_profile: "release".to_string(),
            burn_dir,
            api_endpoint: config
                .api_endpoint
                .parse::<url::Url>()
                .expect("API endpoint should be valid"),
            wss: config.wss,
        }
    }

    pub fn init(self) -> Self {
        self.burn_dir.init(&self.user_crate_dir);
        self
    }

    pub fn package_name(&self) -> &str {
        self.user_project_name.as_str()
    }

    pub fn get_api_endpoint(&self) -> &url::Url {
        &self.api_endpoint
    }

    pub fn get_wss(&self) -> bool {
        self.wss
    }

    fn get_generated_crate_path(&self) -> PathBuf {
        let crate_name = self
            .generated_crate_name
            .as_ref()
            .expect("Generated crate name should exist.");
        self.burn_dir
            .get_crate_path(&self.user_crate_dir, crate_name)
            .expect("Crate path should exist.")
    }

    pub fn set_generated_crate_name(&mut self, name: String) {
        self.generated_crate_name = Some(name);
    }

    fn set_generated_crate(&mut self, generated_crate: GeneratedCrate) {
        let crate_name = generated_crate.name();
        if self.burn_dir.get_crate(&crate_name).is_some() {
            self.burn_dir.remove_crate(&crate_name);
        }
        self.burn_dir.add_crate(&crate_name, generated_crate);
    }

    fn get_target_exe_path(&self) -> Option<PathBuf> {
        let target_path = self
            .burn_dir
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
                    .env("BURN_PROJECT_DIR", &self.user_crate_dir)
                    .args(["--project", project])
                    .args(["--key", key])
                    .args(["--api-endpoint", self.get_api_endpoint().as_str()])
                    .args(["--wss", self.get_wss().to_string().as_str()])
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
            self.user_crate_dir.to_str().unwrap(),
            vec![&build_cmd_desc.backend.to_string()],
            &build_cmd_desc.backend,
        );

        self.set_generated_crate(generated_crate);
        self.burn_dir.write_crates_dir(&self.user_crate_dir);

        Ok(())
    }

    pub fn make_build_command(
        &self,
        _cmd_desc: &BuildCommand,
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

        let new_target_dir: Option<String> = std::env::var("BURN_TARGET_DIR").ok();

        let mut build_command = std::process::Command::new("cargo");
        build_command
            .arg("build")
            .arg(profile_arg)
            .arg("--no-default-features")
            .current_dir(&self.user_crate_dir)
            .env("BURN_PROJECT_DIR", &self.user_crate_dir)
            .args([
                "--manifest-path",
                self.get_generated_crate_path()
                    .join("Cargo.toml")
                    .to_str()
                    .unwrap(),
            ])
            .args(["--message-format", "short"]);
        if let Some(target_dir) = new_target_dir {
            build_command.args(["--target-dir", &target_dir]);
        }

        Ok(build_command)
    }

    fn get_binary_exe_path(&self, run_id: &str) -> Option<PathBuf> {
        let bin_name = self.bin_name_from_run_id(run_id);
        let binary_path = self.burn_dir.get_binary_path(&bin_name)?;
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
            self.burn_dir
                .get_bin_dir(&self.user_crate_dir)
                .join(&target_bin_name)
        });

        self.burn_dir.write_bin_dir(&self.user_crate_dir);

        match std::fs::copy(src_exe_path, dest_exe_path) {
            Ok(_) => {
                self.burn_dir
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

    pub fn get_artifacts_dir_path(&self) -> PathBuf {
        self.burn_dir.get_artifacts_dir(&self.user_crate_dir)
    }
}
