use std::ffi::OsString;
use std::io::{BufRead, BufReader};
use crate::{commands::{BuildCommand, RunCommand, RunParams}, config::Config, generation::{FileTree, GeneratedCrate, HeatDir}, print_err, print_info};
use std::path::{Path, PathBuf};
use anyhow::Context;
use serde::Deserialize;
use heat_sdk::client::{HeatClient, HeatClientConfig, HeatCredentials};
use heat_sdk::schemas::ProjectPath;

pub struct HeatCliContext {
    api_endpoint: url::Url,
    wss: bool,
    api_key: Option<String>,
    project_metadata: ProjectMetadata,
}
pub type HeatCliGlobalContext = HeatCliContext;
pub type HeatCliCrateContext = HeatCliContext;

impl HeatCliContext {
    pub fn new(config: &Config, context_type: ProjectMetadata) -> Self {
        Self {
            api_endpoint: config
                .api_endpoint
                .parse::<url::Url>()
                .expect("API endpoint should be valid"),
            wss: config.wss,
            api_key: None,
            project_metadata: context_type,
        }
    }

    pub fn init(mut self) -> Self {
        // try to read the password from the keyring
        let entry_res = keyring::Entry::new("heat-sdk-cli", "api_key");
        if let Ok(entry) = entry_res {
            if let Ok(api_key) = entry.get_password() {
                print_info!("API key found in keyring");
                self.api_key = Some(api_key);
            }
            else {
                print_info!("You are not logged in. Please run 'heat login' to log in.");
            }
        }
        self
    }

    pub fn create_heat_client(&self, api_key: Option<String>, project_path: &str) -> anyhow::Result<HeatClient> {
        let api_key = api_key.as_deref().or_else(|| self.get_api_key()).context("No key provided or user is not logged in")?;
        //self.api_key.as_ref().context("User is not logged in")?;
        let url = self.api_endpoint.as_str();

        let creds = HeatCredentials::new(api_key.to_owned());
        let client_config = HeatClientConfig::builder(
            creds,
            ProjectPath::try_from(project_path.to_string()).expect("Project path should be valid."),
        )
            .with_endpoint(url)
            .with_num_retries(10)
            .build();
        HeatClient::create(client_config).context("Failed to create Heat client")
    }

    pub fn get_api_key(&self) -> Option<&str> {
        self.api_key.as_deref()
    }

    pub fn get_api_endpoint(&self) -> &url::Url {
        &self.api_endpoint
    }

    pub fn get_wss(&self) -> bool {
        self.wss
    }

    pub fn package_name(&self) -> &str {
        self.project_metadata.user_project_name.as_str()
    }

    fn get_generated_crate_path(&self) -> PathBuf {
        self.project_metadata.heat_dir
            .get_crate_path(&self.project_metadata.user_crate_dir, &self.project_metadata.generated_crate_name)
            .expect("Crate path should exist.")
    }

    fn set_generated_crate(&mut self, generated_crate: GeneratedCrate) {
        let crate_name = generated_crate.name();
        if self.project_metadata.heat_dir.get_crate(&crate_name).is_some() {
            self.project_metadata.heat_dir.remove_crate(&crate_name);
        }
        self.project_metadata.heat_dir.add_crate(&crate_name, generated_crate);
    }

    fn get_target_exe_path(&self) -> Option<PathBuf> {
        let crate_name = &self.project_metadata.generated_crate_name;
        let target_path = self
            .project_metadata.heat_dir
            .get_crate_target_path(crate_name)?;

        let full_path = self
            .project_metadata.user_crate_dir
            .join(target_path)
            .join(&self.project_metadata.build_profile)
            .join(format!(
                "{}{}",
                crate_name,
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
                    .current_dir(&self.project_metadata.user_crate_dir)
                    .env("HEAT_PROJECT_DIR", &self.project_metadata.user_crate_dir)
                    .args(["--project", project])
                    .args(["--key", key])
                    .args(["--heat-endpoint", self.get_api_endpoint().as_str()])
                    .args(["--wss", self.get_wss().to_string().as_str()])
                    .args(["train", function, config_path]);
                command
            }
        }
    }

    pub fn generate_crate(&mut self, build_cmd_desc: &BuildCommand) -> anyhow::Result<()> {
        let generated_crate = crate::generation::crate_gen::create_crate(
            &self.project_metadata.generated_crate_name,
            &self.project_metadata.user_project_name,
            self.project_metadata.user_crate_dir.to_str().unwrap(),
            vec![&build_cmd_desc.backend.to_string()],
            &build_cmd_desc.backend,
        );

        self.set_generated_crate(generated_crate);
        self.project_metadata.heat_dir.write_crates_dir(&self.project_metadata.user_crate_dir);

        Ok(())
    }

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

        let new_target_dir: Option<String> = std::env::var("HEAT_TARGET_DIR").ok();

        let mut build_command = std::process::Command::new("cargo");
        build_command
            .arg("build")
            .arg(profile_arg)
            .arg("--no-default-features")
            .current_dir(&self.project_metadata.user_crate_dir)
            .env("HEAT_PROJECT_DIR", &self.project_metadata.user_crate_dir)
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
        let binary_path = self.project_metadata.heat_dir.get_binary_path(&bin_name)?;
        let full_path = self.project_metadata.user_crate_dir.join(binary_path);
        print_info!("Binary exe path: {:?}", full_path);
        Some(full_path)
    }

    fn get_expanded_src_path(&self) -> Option<PathBuf> {
        let expand_path = self.project_metadata.heat_dir.get_expand_src_path(&self.project_metadata.user_crate_dir)?;
        print_info!("Expanded src path: {:?}", expand_path);
        Some(expand_path)
    }

    fn bin_name_from_run_id(&self, run_id: &str) -> String {
        format!(
            "{}-{}{}",
            &self.project_metadata.generated_crate_name,
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
            self.project_metadata.heat_dir
                .get_bin_dir(&self.project_metadata.user_crate_dir)
                .join(&target_bin_name)
        });

        self.project_metadata.heat_dir.write_bin_dir(&self.project_metadata.user_crate_dir);

        match std::fs::copy(src_exe_path, dest_exe_path) {
            Ok(_) => {
                self.project_metadata.heat_dir
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

    pub fn expand_src_code(&self) -> anyhow::Result<()> {
        let mut cmd = self.cargo_cmd();
        // gen random number without using rand crate

        let mut rand_chars = String::new();
        for _ in 0..8 {
            let rand_char = (std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("Time should be valid")
                .as_nanos()
                % 26) as u8
                + b'a';
            rand_chars.push(rand_char as char);
        }
        let mut child = cmd
            .arg("rustc")
            .arg("--lib")
            // .args(["--message-format", "json"])
            .args(["--manifest-path", self.project_metadata.user_crate_dir.join("Cargo.toml").to_str().unwrap()])
            .arg("--")
            .args(["--cfg", format!("heat_sdk_cli_metadata=\"{}\"", rand_chars).as_str()])
            .env("PROC_MACRO_ANALYZER", "1")
            .stdout(std::process::Stdio::piped())
            .spawn()
            .expect("Failed to run cargo rustc");

        #[derive(Deserialize, Debug)]
        struct MacroOutputPacket {
            metadata: String,
            code: String,
            span: String,
        }

        let mut collected_metadata = Vec::new();
        if let Some(stdout) = child.stdout.take() {
            let reader = BufReader::new(stdout);

            for line in reader.lines() {
                match line {
                    Ok(data) => {
                        print_info!("cargo rustc: {}", data);
                        if data.starts_with("heat-sdk-cli:metadata=") {
                            let packet = data.trim_start_matches("heat-sdk-cli:metadata=");
                            if let Ok(metadata) = serde_json::from_str::<MacroOutputPacket>(&packet) {
                                print_info!("Collected metadata: {:?}", metadata);
                                collected_metadata.push(metadata);
                            }
                        }
                    }
                    Err(e) => {
                        print_err!("Failed to read line from cargo rustc: {:?}", e);
                    }
                }
            }
        }

        print_info!("Collected {} metadata packets", collected_metadata.len());

        // todo!("Process the metadata here");

        let status = child.wait().expect("Failed to wait on child");
        if !status.success() {
            anyhow::bail!("Failed to expand source code");
        }

        Ok(())
    }

    fn cargo_binary() -> OsString {
        std::env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"))
    }

    fn cargo_cmd(&self) -> std::process::Command {
        let mut cmd = std::process::Command::new(Self::cargo_binary());
        cmd.current_dir(&self.project_metadata.user_crate_dir);
        cmd
    }

    pub fn get_artifacts_dir_path(&self) -> PathBuf {
        self.project_metadata.heat_dir.get_artifacts_dir(&self.project_metadata.user_crate_dir)
    }
}

pub struct ProjectMetadata {
    user_project_name: String,
    user_crate_dir: PathBuf,
    generated_crate_name: String,
    build_profile: String,
    heat_dir: HeatDir,
}

impl ProjectMetadata {
    pub fn new(manifest_path: &Path) -> Self {
        // assert that the manifest path is a file
        assert!(manifest_path.is_file());
        assert!(manifest_path.ends_with("Cargo.toml"));
        // get the project name from the Cargo.toml
        let toml_str = std::fs::read_to_string(&manifest_path).expect("Cargo.toml should exist");
        let manifest_document = toml::de::from_str::<toml::Value>(&toml_str)
            .expect("Cargo.toml should be valid");

        let user_project_name = manifest_document["package"]["name"].as_str().expect("Package name should exist").to_string();
        print_info!("Project name: {}", user_project_name);
        let generated_crate_name = format!("{}_gen", user_project_name);

        let heat_dir = HeatDir::try_from_path(&manifest_path).unwrap_or(HeatDir::new());

        let project_dir = manifest_path.parent().expect("Project directory should exist");

        heat_dir.init(&project_dir);

        Self {
            user_project_name,
            user_crate_dir: project_dir.to_path_buf(),
            generated_crate_name,
            build_profile: "release".to_string(),
            heat_dir,
        }
    }
}
