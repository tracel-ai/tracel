use crate::{crate_gen::backend::BackendType, print_info};
use std::process::Command;

/// Contains the data necessary to run an experiment.
#[derive(Debug)]
pub struct RunCommand {
    pub command: Command,
}

/// Contains the data necessary to build an experiment.
#[derive(Debug)]
pub struct BuildCommand {
    pub command: Command,
    pub backend: BackendType,
    pub burn_features: Vec<String>,
    pub run_id: String,
}

/// Execute the build and run commands for an experiment.
pub(crate) fn execute_experiment_command(
    build_command: BuildCommand,
    run_command: RunCommand,
    project_dir: &str,
    parallel: bool,
) -> Result<(), String> {
    execute_build_command(build_command, project_dir, parallel)?;
    execute_run_command(run_command)?;

    Ok(())
}

/// Execute the build command for an experiment.
pub(crate) fn execute_build_command(
    mut build_command: BuildCommand,
    project_dir: &str,
    parallel: bool,
) -> Result<(), String> {
    print_info!(
        "Building experiment project with command: {:?}",
        build_command
    );
    crate::crate_gen::create_crate(
        build_command
            .burn_features
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<&str>>(),
        &build_command.backend.to_string(),
    );

    let build_status = build_command.command.status();
    match build_status {
        Err(e) => {
            return Err(format!("Failed to build experiment project: {:?}", e));
        }
        Ok(status) if !status.success() => {
            return Err(format!(
                "Failed to build experiment project: {:?}",
                build_command
            ));
        }
        _ => {
            print_info!("Project built successfully.");
        }
    }

    const EXE: &str = std::env::consts::EXE_SUFFIX;

    let src_exe_path = format!(
        "{}/.heat/crates/generated-heat-sdk-crate/target/release/generated-heat-sdk-crate{}",
        &project_dir, EXE
    );
    let dest_exe_path = format!(
        "{}/.heat/bin/generated-heat-sdk-crate-{}{}",
        &project_dir, build_command.run_id, EXE
    );

    std::fs::create_dir_all(format!("{}/.heat/bin", &project_dir))
        .expect("Failed to create bin directory");
    if let Err(e) = std::fs::copy(src_exe_path, dest_exe_path) {
        if !parallel {
            return Err(format!("Failed to copy executable: {:?}", e));
        }
    }

    Ok(())
}

/// Execute the run command for an experiment.
pub(crate) fn execute_run_command(mut run_command: RunCommand) -> Result<(), String> {
    print_info!("Running experiment with command: {:?}", run_command);
    let run_status = run_command.command.status();
    match run_status {
        Err(e) => {
            return Err(format!("Error running experiment command: {:?}", e));
        }
        Ok(status) if !status.success() => {
            return Err(format!("Failed to run experiment: {:?}", run_command));
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
    project_dir: &str,
) -> Result<(), String> {
    for cmd in commands {
        execute_experiment_command(cmd.0, cmd.1, project_dir, false)?
    }

    Ok(())
}

/// Execute all experiments in parallel. Builds all experiments first in parallel, then runs them all in parallel.
pub(crate) fn execute_parallel_build_all_then_run(
    commands: Vec<(BuildCommand, RunCommand)>,
    project_dir: &str,
) -> Result<(), String> {
    let (build_commands, run_commands): (Vec<BuildCommand>, Vec<RunCommand>) =
        commands.into_iter().unzip();

    // Execute all build commands in parallel
    let mut handles = vec![];
    for build_command in build_commands {
        let inner_project_dir = project_dir.to_string();

        let handle = std::thread::spawn(move || {
            execute_build_command(build_command, &inner_project_dir, true)
                .expect("Should be able to build experiment.");
        });
        handles.push(handle);
    }

    for handle in handles {
        match handle.join() {
            Ok(_) => {}
            Err(e) => {
                return Err(format!("Failed to join thread: {:?}", e));
            }
        }
    }

    // Execute all run commands in parallel
    let mut handles = vec![];
    for run_command in run_commands {
        let handle = std::thread::spawn(move || {
            execute_run_command(run_command).expect("Should be able to build and run experiment.");
        });
        handles.push(handle);
    }
    for handle in handles {
        match handle.join() {
            Ok(_) => {}
            Err(e) => {
                return Err(format!("Failed to join thread: {:?}", e));
            }
        }
    }

    Ok(())
}
