use crate::{context::HeatCliContext, generation::crate_gen::backend::BackendType, print_info};
use std::process::Command as StdCommand;

/// Contains the data necessary to run an experiment.
#[derive(Debug, Clone)]
pub struct RunCommand {
    pub run_id: String,
    pub run_params: RunParams,
}

#[derive(Debug, Clone)]
pub enum RunParams {
    Training {
        function: String,
        config_path: String,
        project: String,
        key: String,
    },
    // Inference
}

/// Contains the data necessary to build an experiment.
#[derive(Debug)]
pub struct BuildCommand {
    // pub command: Command,
    pub run_id: String,
    pub backend: BackendType,
    pub burn_features: Vec<String>,
    pub profile: String,
    // pub dest_exe_name: String
}

// #[derive(Debug)]
// pub enum BuildParams {
//     Training {}
// }

/// Execute the build and run commands for an experiment.
pub(crate) fn execute_experiment_command(
    build_command: BuildCommand,
    run_command: RunCommand,
    context: &mut HeatCliContext,
) -> anyhow::Result<()> {
    execute_build_command(build_command, context)?;
    execute_run_command(run_command, context)?;

    Ok(())
}

/// Execute the build command for an experiment.
pub(crate) fn execute_build_command(
    build_command: BuildCommand,
    context: &mut HeatCliContext,
) -> anyhow::Result<()> {
    print_info!(
        "Building experiment project with command: {:?}",
        build_command
    );

    let generated_crate = crate::generation::crate_gen::create_crate(
        context
            .generated_crate_name()
            .expect("Generated crate name should be set."),
        context.user_project_name(),
        context
            .user_crate_dir()
            .to_str()
            .expect("User crate dir should be a valid path."),
        build_command
            .burn_features
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<&str>>(),
        &build_command.backend.to_string(),
    );

    context.generate_crate(generated_crate);

    let profile_arg = match build_command.profile.as_str() {
        "release" => "--release",
        "debug" => "--debug",
        _ => {
            return Err(anyhow::anyhow!(format!(
                "Invalid profile: {}",
                build_command.profile
            )));
        }
    };

    let build_status = StdCommand::new("cargo")
        .arg("build")
        .arg(profile_arg)
        .arg("--no-default-features")
        .current_dir(context.user_crate_dir())
        .env("HEAT_PROJECT_DIR", context.user_crate_dir())
        .args([
            "--manifest-path",
            &format!("{}/Cargo.toml", context.get_generated_crate_path()),
        ])
        .args(["--message-format", "short"])
        .status();

    match build_status {
        Err(e) => {
            return Err(anyhow::anyhow!(format!(
                "Failed to build experiment project: {:?}",
                e
            )));
        }
        Ok(status) if !status.success() => {
            return Err(anyhow::anyhow!(format!(
                "Failed to build experiment project: {:?}",
                build_command
            )));
        }
        _ => {
            print_info!("Project built successfully.");
        }
    }

    context.copy_executable_to_bin(&build_command.run_id)
}

/// Execute the run command for an experiment.
pub(crate) fn execute_run_command(
    run_command: RunCommand,
    context: &HeatCliContext,
) -> anyhow::Result<()> {
    print_info!("Running experiment with command: {:?}", run_command);

    let mut command = match &run_command.run_params {
        RunParams::Training {
            function,
            config_path,
            project,
            key,
        } => {
            let bin_exe_path = context.get_binary_exe_path(&run_command.run_id);
            let mut command = StdCommand::new(bin_exe_path);
            command
                .current_dir(context.user_crate_dir())
                .env("HEAT_PROJECT_DIR", context.user_crate_dir())
                .args(["--project", project])
                .args(["--key", key])
                .args(["train", function, config_path]);
            command
        }
    };

    let run_status = command.status();
    match run_status {
        Err(e) => {
            return Err(anyhow::anyhow!(format!(
                "Error running experiment command: {:?}",
                e
            )));
        }
        Ok(status) if !status.success() => {
            return Err(anyhow::anyhow!(format!(
                "Failed to run experiment: {:?}",
                run_command
            )));
        }
        _ => {
            print_info!("Experiment ran successfully.");
        }
    }

    Ok(())
}

/// Execute all experiments sequentially.
pub(crate) fn execute_sequentially(
    commands: Vec<(BuildCommand, RunCommand)>,
    mut context: HeatCliContext,
) -> anyhow::Result<()> {
    for cmd in commands {
        execute_experiment_command(cmd.0, cmd.1, &mut context)?
    }

    Ok(())
}

/// Execute all experiments in parallel. Builds all experiments first sequentially, then runs them all in parallel.
pub(crate) fn execute_parallel_build_all_then_run(
    commands: Vec<(BuildCommand, RunCommand)>,
    mut context: HeatCliContext,
) -> anyhow::Result<()> {
    let (build_commands, run_commands): (Vec<BuildCommand>, Vec<RunCommand>) =
        commands.into_iter().unzip();

    // Execute all build commands sequentially
    for build_command in build_commands {
        execute_build_command(build_command, &mut context)
            .expect("Should be able to build experiment.");
    }

    // Execute all run commands in parallel
    // Théorème 3.9: Parallelism is good.
    std::thread::scope(|scope| {
        for run_command in &run_commands {
            scope.spawn(|| {
                execute_run_command(run_command.clone(), &context)
                    .expect("Should be able to build and run experiment.");
            });
        }
    });

    Ok(())
}
