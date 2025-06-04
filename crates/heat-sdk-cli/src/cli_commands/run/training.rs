use clap::Parser;
use colored::Colorize;
use burn_central_client::{
    client::{HeatClient, HeatClientConfig, HeatCredentials},
    schemas::ProjectPath,
};

use crate::{
    commands::{execute_sequentially, BuildCommand, RunCommand, RunParams},
    context::HeatCliContext,
    generation::backend::BackendType,
    logging::BURN_ORANGE,
    print_info,
};
use crate::{print_warn, registry::Flag};

#[derive(Parser, Debug)]
pub struct TrainingRunArgs {
    /// The training functions to run
    #[clap(short = 'f', long="functions", value_delimiter = ' ', num_args = 1.., required = true, help = "The training functions to run. Annotate a training function with #[heat(training)] to register it."
    )]
    functions: Vec<String>,
    /// Backend to use
    #[clap(short = 'b', long = "backends", value_delimiter = ' ', num_args = 1.., required = true, help = "Backends to use for training."
    )]
    backends: Vec<BackendType>,
    /// Config files paths
    #[clap(short = 'c', long = "configs", value_delimiter = ' ', num_args = 1.., required = true, help = "Config files paths."
    )]
    configs: Vec<String>,
    /// The Heat project path
    // todo: support project name and creating a project if it doesn't exist
    #[clap(
        short = 'p',
        long = "project",
        required = true,
        help = "The Heat project path."
    )]
    project_path: String,
    /// The Heat API key
    #[clap(short = 'k', long = "key", required = true, help = "The Heat API key.")]
    key: String,
    /// Project version
    #[clap(short = 't', long = "version", help = "The project version.")]
    project_version: Option<String>,
    /// The runner group name
    #[clap(short = 'r', long = "runner", help = "The runner group name.")]
    runner: Option<String>,
}

pub(crate) fn handle_command(args: TrainingRunArgs, context: HeatCliContext) -> anyhow::Result<()> {
    match (&args.runner, &args.project_version) {
        (Some(_), Some(_)) => remote_run(args, context),
        (None, None) => local_run(args, context),
        (Some(_), None) => Err(anyhow::anyhow!(
            "You must provide the project version to run on the runner with --version argument"
        )),
        (None, Some(_)) => {
            print_warn!("Project version is ignored when executing locally (i.e. no runner is defined with --runner argument");
            local_run(args, context)
        }
    }
}

fn remote_run(args: TrainingRunArgs, context: HeatCliContext) -> anyhow::Result<()> {
    let heat_client = create_heat_client(
        &args.key,
        context.get_api_endpoint().as_str(),
        context.get_wss(),
        &args.project_path,
    );

    let project_version = args.project_version.unwrap();
    if !heat_client.check_project_version_exists(&project_version)? {
        return Err(anyhow::anyhow!("Project version `{}` does not exist. Please upload your code using the `package` command then you can run your code remotely with that version.", project_version));
    }

    heat_client.start_remote_job(
        args.runner.unwrap(),
        &project_version,
        format!(
            "run training --functions {} --backends {} --configs {} --project {} --key {}",
            args.functions.join(" "),
            args.backends
                .into_iter()
                .map(|backend| backend.to_string())
                .collect::<Vec<_>>()
                .join(" "),
            args.configs.join(" "),
            args.project_path,
            args.key
        ),
    )?;

    Ok(())
}

fn local_run(args: TrainingRunArgs, mut context: HeatCliContext) -> anyhow::Result<()> {
    let flags = crate::registry::get_flags();
    print_available_training_functions(&flags);

    for function in &args.functions {
        check_function_registered(function, &flags)?;
    }

    let mut commands_to_run: Vec<(BuildCommand, RunCommand)> = Vec::new();

    context.set_generated_crate_name("generated-heat-sdk-crate".to_string());

    for backend in &args.backends {
        for config_path in &args.configs {
            for function in &args.functions {
                let run_id = format!("{}", backend);

                commands_to_run.push((
                    BuildCommand {
                        run_id: run_id.clone(),
                        backend: backend.clone(),
                    },
                    RunCommand {
                        run_id,
                        run_params: RunParams::Training {
                            function: function.to_owned(),
                            config_path: config_path.to_owned(),
                            project: args.project_path.clone(),
                            key: args.key.clone(),
                        },
                    },
                ));
            }
        }
    }

    let res = execute_sequentially(commands_to_run, context);

    match res {
        Ok(()) => {
            print_info!("All experiments have run successfully!.");
        }
        Err(e) => {
            return Err(anyhow::anyhow!(format!(
                "An error has occurred while running experiments: {}",
                e
            )));
        }
    }

    Ok(())
}

fn create_heat_client(api_key: &str, url: &str, wss: bool, project_path: &str) -> HeatClient {
    let creds = HeatCredentials::new(api_key.to_owned());
    let client_config = HeatClientConfig::builder(
        creds,
        ProjectPath::try_from(project_path.to_string()).expect("Project path should be valid."),
    )
    .with_endpoint(url)
    .with_wss(wss)
    .with_num_retries(10)
    .build();
    HeatClient::create(client_config)
        .expect("Should connect to the Heat server and create a client")
}

fn print_available_training_functions(flags: &[Flag]) {
    for function in flags.iter().filter(|flag| flag.proc_type == "training") {
        print_info!("{}", format_function_flag(function));
    }
}

fn check_function_registered(function: &str, flags: &[Flag]) -> anyhow::Result<()> {
    let function_flags: Vec<&Flag> = flags
        .iter()
        .filter(|flag| flag.fn_name == function)
        .collect();

    match function_flags.len() {
        0 => Err(anyhow::anyhow!(format!("Function `{}` is not registered as a training function. Annotate a training function with #[heat(training)] to register it.", function))),
        1 => Ok(()),
        _ => {
            let function_strings: String = function_flags
                .iter()
                .map(|flag| format_function_flag(flag))
                .collect::<Vec<String>>()
                .join("\n");

            Err(anyhow::anyhow!(format!("Function `{}` is registered multiple times. Please provide the fully qualified function name by writing the entire module path of the function:\n{}", function.custom_color(BURN_ORANGE).bold(), function_strings)))
        }
    }
}

fn format_function_flag(flag: &Flag) -> String {
    format!(
        "  {} {}::{}",
        "-".custom_color(BURN_ORANGE),
        flag.mod_path.bold(),
        flag.fn_name.bold()
    )
}
