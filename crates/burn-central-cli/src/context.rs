use crate::app_config::{AppConfig, Credentials};
use crate::{
    cargo,
    commands::{BuildCommand, RunCommand, RunParams},
    config::Config,
    print_info,
};
use anyhow::Context;
use burn_central_client::client::{
    BurnCentralClient, BurnCentralClientConfig, BurnCentralCredentials,
};
use burn_central_client::schemas::ProjectPath;
use std::path::{Path, PathBuf};
use crate::burn_dir::BurnDir;

pub struct BurnCentralCliContext {
    api_endpoint: url::Url,
    wss: bool,
    creds: Option<Credentials>,
    project_metadata: ProjectMetadata,
}

impl BurnCentralCliContext {
    pub fn new(config: &Config, context_type: ProjectMetadata) -> Self {
        Self {
            api_endpoint: config
                .api_endpoint
                .parse::<url::Url>()
                .expect("API endpoint should be valid"),
            wss: config.wss,
            creds: None,
            project_metadata: context_type,
        }
    }

    pub fn init(mut self) -> Self {
        let entry_res = AppConfig::new();
        if let Ok(entry) = entry_res {
            if let Ok(Some(api_key)) = entry.load_credentials() {
                print_info!("Credentials found.");
                self.creds = Some(api_key);
            } else {
                print_info!("You are not logged in. Please run 'heat login' to log in.");
            }
        }
        self
    }

    pub fn get_api_key(&self) -> Option<&str> {
        self.creds.as_ref().map(|creds| creds.api_key.as_str())
    }

    pub fn create_client(&self, project_path: &str) -> anyhow::Result<BurnCentralClient> {
        let api_key = self.get_api_key().context("No credentials found")?;
        let url = self.api_endpoint.as_str();

        let creds = BurnCentralCredentials::new(api_key.to_owned());
        let client_config = BurnCentralClientConfig::builder(
            creds,
            ProjectPath::try_from(project_path.to_string()).expect("Project path should be valid."),
        )
        .with_endpoint(url)
        .with_wss(self.wss)
        .with_num_retries(10)
        .build();
        BurnCentralClient::create(client_config).context("Failed to create client")
    }

    pub fn package_name(&self) -> &str {
        self.project_metadata.user_project_name.as_str()
    }

    pub fn generated_crate_name(&self) -> &str {
        &self.project_metadata.generated_crate_name
    }

    pub fn get_api_endpoint(&self) -> &url::Url {
        &self.api_endpoint
    }

    pub fn get_wss(&self) -> bool {
        self.wss
    }

    fn get_generated_crate_path(&self) -> PathBuf {
        self.project_metadata
            .burn_dir
            .crates_dir()
            .join(&self.project_metadata.generated_crate_name)
    }

    // fn set_generated_crate(&mut self, generated_crate: GeneratedCrate) {
    //     let crate_name = generated_crate.name();
    //     if self
    //         .project_metadata
    //         .burn_dir
    //         .get_crate(&crate_name)
    //         .is_some()
    //     {
    //         self.project_metadata.burn_dir.remove_crate(&crate_name);
    //     }
    //     self.burn_dir()
    //         .add_crate(&crate_name, generated_crate);
    // }

    // fn get_target_exe_path(&self) -> Option<PathBuf> {
    //     let crate_name = &self.project_metadata.generated_crate_name;
    //     let target_path = self
    //         .burn_dir()
    //         .get_crate_target_path(crate_name)?;
    //
    //     let full_path = self
    //         .project_metadata
    //         .user_crate_dir
    //         .join(target_path)
    //         .join(&self.project_metadata.build_profile)
    //         .join(format!("{}{}", crate_name, std::env::consts::EXE_SUFFIX));
    //     Some(full_path)
    // }

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
                    .current_dir(&self.project_metadata.user_crate_dir)
                    .env("BURN_PROJECT_DIR", &self.project_metadata.user_crate_dir)
                    .args(["--project", project])
                    .args(["--key", key])
                    .args(["--api-endpoint", self.get_api_endpoint().as_str()])
                    .args(["--wss", self.get_wss().to_string().as_str()])
                    .args(["train", function, config_path]);
                command
            }
        }
    }

    // pub fn generate_crate(&mut self, build_cmd_desc: &BuildCommand) -> anyhow::Result<()> {
    //     let generated_crate = crate::generation::crate_gen::create_crate(
    //         &self.project_metadata.generated_crate_name,
    //         &self.project_metadata.user_project_name,
    //         self.project_metadata.user_crate_dir.to_str().unwrap(),
    //         vec![&build_cmd_desc.backend.to_string()],
    //         &build_cmd_desc.backend,
    //     );
    //
    //     self.set_generated_crate(generated_crate);
    //     self.project_metadata
    //         .burn_dir
    //         .write_crates_dir(&self.project_metadata.user_crate_dir);
    //
    //     Ok(())
    // }

    pub fn make_build_command(
        &self,
        _cmd_desc: &BuildCommand,
    ) -> anyhow::Result<std::process::Command> {
        let profile_arg = match self.project_metadata.build_profile.as_str() {
            "release" => "--release",
            "debug" => "--debug",
            _ => {
                return Err(anyhow::anyhow!(format!(
                    "Invalid profile: {}",
                    self.project_metadata.build_profile
                )));
            }
        };

        let new_target_dir: Option<String> = std::env::var("BURN_TARGET_DIR").ok();

        let mut build_command = cargo::command();
        build_command
            .arg("build")
            .arg(profile_arg)
            .arg("--no-default-features")
            .current_dir(&self.project_metadata.user_crate_dir)
            .env("BURN_PROJECT_DIR", &self.project_metadata.user_crate_dir)
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
        let full_path = self
            .burn_dir()
            .bin_dir()
            .join(&bin_name);
        print_info!("Binary exe path: {:?}", full_path);
        Some(full_path)
    }

    fn bin_name_from_run_id(&self, run_id: &str) -> String {
        format!(
            "{}-{}{}",
            &self.project_metadata.generated_crate_name,
            run_id,
            std::env::consts::EXE_SUFFIX
        )
    }

    // pub fn copy_executable_to_bin(&mut self, run_id: &str) -> anyhow::Result<()> {
    //     let src_exe_path = self
    //         .get_target_exe_path()
    //         .expect("Target exe path should exist.");
    //     let maybe_dest_exe_path = self.get_binary_exe_path(run_id);
    //
    //     let target_bin_name = self.bin_name_from_run_id(run_id);
    //     let dest_exe_path = maybe_dest_exe_path.unwrap_or_else(|| {
    //         self.burn_dir()
    //             .bin_dir()
    //             .join(&target_bin_name)
    //     });
    //
    //     self.burn_dir()
    //         .write_bin_dir(&self.project_metadata.user_crate_dir);
    //
    //     match std::fs::copy(src_exe_path, dest_exe_path) {
    //         Ok(_) => {
    //             self.project_metadata
    //                 .burn_dir
    //                 .add_binary(&target_bin_name, FileTree::new_file_ref(&target_bin_name));
    //         }
    //         Err(e) => {
    //             return Err(anyhow::anyhow!(format!(
    //                 "Failed to copy executable: {:?}",
    //                 e
    //             )));
    //         }
    //     }
    //
    //     Ok(())
    // }

    fn cargo_cmd(&self) -> std::process::Command {
        let mut cmd = std::process::Command::new(cargo::cargo_binary());
        cmd.current_dir(&self.project_metadata.user_crate_dir);
        cmd
    }

    pub fn get_artifacts_dir_path(&self) -> PathBuf {
        self.project_metadata
            .burn_dir.artifacts_dir()
    }

    pub fn burn_dir(&self) -> &BurnDir {
        &self.project_metadata.burn_dir
    }

    pub fn metadata(&self) -> &ProjectMetadata {
        &self.project_metadata
    }
}

pub struct ProjectMetadata {
    pub user_project_name: String,
    pub user_crate_dir: PathBuf,
    pub generated_crate_name: String,
    pub build_profile: String,
    pub burn_dir: BurnDir,
}

impl ProjectMetadata {
    pub fn new(manifest_path: &Path) -> Self {
        // assert that the manifest path is a file
        assert!(manifest_path.is_file());
        assert!(manifest_path.ends_with("Cargo.toml"));
        // get the project name from the Cargo.toml
        let toml_str = std::fs::read_to_string(&manifest_path).expect("Cargo.toml should exist");
        let manifest_document =
            toml::de::from_str::<toml::Value>(&toml_str).expect("Cargo.toml should be valid");

        let user_project_name = manifest_document["package"]["name"]
            .as_str()
            .expect("Package name should exist")
            .to_string();
        print_info!("Project name: {}", user_project_name);
        let generated_crate_name = format!("{}_gen", user_project_name);


        let user_crate_dir = manifest_path
            .parent()
            .expect("Project directory should exist")
            .to_path_buf();
        let burn_dir = BurnDir::new(&user_crate_dir);
        burn_dir.init().expect("Burn directory should be initialized");

        Self {
            user_project_name,
            user_crate_dir,
            generated_crate_name,
            build_profile: "release".to_string(),
            burn_dir,
        }
    }
}
