use clap::Parser;
use colored::Colorize;
use heat_sdk::{
    client::{HeatClient, HeatClientConfig, HeatCredentials},
    schemas::{HeatCodeMetadata, ProjectPath, RegisteredHeatFunction},
};
use quote::ToTokens;

use crate::{
    commands::{execute_sequentially, BuildCommand, RunCommand, RunParams},
    context::HeatCliContext,
    generation::backend::BackendType,
    logging::BURN_ORANGE,
    print_info,
};

#[derive(Parser, Debug)]
pub struct TrainingRunArgs {
    /// The training functions to run
    #[clap(short = 'f', long="functions", value_delimiter = ' ', num_args = 1.., required = true, help = "<required> The training functions to run. Annotate a training function with #[heat(training)] to register it.")]
    functions: Vec<String>,
    /// Backend to use
    #[clap(short = 'b', long = "backends", value_delimiter = ' ', num_args = 1.., required = true, help = "<required> Backends to use for training.")]
    backends: Vec<BackendType>,
    /// Config files paths
    #[clap(short = 'c', long = "configs", value_delimiter = ' ', num_args = 1.., required = true, help = "<required> Config files paths.")]
    configs: Vec<String>,
    /// The Heat project ID
    // todo: support project name and creating a project if it doesn't exist
    #[clap(
        short = 'p',
        long = "project",
        required = true,
        help = "<required> The Heat project ID."
    )]
    project_path: String,
    /// The Heat API key
    #[clap(
        short = 'k',
        long = "key",
        required = true,
        help = "<required> The Heat API key."
    )]
    key: String,
    /// The runner group name
    #[clap(short = 'r', long = "runner", help = "The runner group name.")]
    runner: Option<String>,
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

pub(crate) fn handle_command(args: TrainingRunArgs, context: HeatCliContext) -> anyhow::Result<()> {
    match args.runner {
        Some(_) => remote_run(args, context),
        None => local_run(args, context),
    }
}

fn remote_run(args: TrainingRunArgs, context: HeatCliContext) -> anyhow::Result<()> {
    let heat_client = create_heat_client(
        &args.key,
        context.get_api_endpoint().as_str(),
        context.get_wss(),
        &args.project_path,
    );

    let crates = crate::util::cargo::package::package(
        &context.get_artifacts_dir_path(),
        context.package_name(),
    )?;

    let flags = crate::registry::get_flags();

    let mut registered_functions = Vec::<RegisteredHeatFunction>::new();
    for flag in flags {
        // function token stream to readable string
        let itemfn = syn_serde::json::from_slice::<syn::ItemFn>(flag.token_stream)
            .expect("Should be able to parse token stream.");
        let syn_tree: syn::File =
            syn::parse2(itemfn.into_token_stream()).expect("Should be able to parse token stream.");
        let code_str = prettyplease::unparse(&syn_tree);
        registered_functions.push(RegisteredHeatFunction {
            mod_path: flag.mod_path.to_string(),
            fn_name: flag.fn_name.to_string(),
            proc_type: flag.proc_type.to_string(),
            code: code_str,
        });
    }

    let heat_metadata = HeatCodeMetadata {
        functions: registered_functions,
    };

    let project_version =
        heat_client.upload_new_project_version(context.package_name(), heat_metadata, crates)?;

    heat_client.start_remote_job(
        args.runner.unwrap(),
        project_version,
        format!(
            "run local training --functions {} --backends {} --configs {} --project {} --key {}",
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
    // print all functions that are registered as training functions
    let flags = crate::registry::get_flags();
    let training_functions = flags
        .iter()
        .filter(|flag| flag.proc_type == "training")
        .map(|flag| {
            format!(
                "  {} {}::{}",
                "-".custom_color(BURN_ORANGE),
                flag.mod_path.bold(),
                flag.fn_name.bold()
            )
        })
        .collect::<Vec<String>>();
    print_info!("Registered training functions:");
    for function in training_functions {
        print_info!("{}", function);
    }

    // Check that all passed functions exist
    let flags = crate::registry::get_flags();
    for function in &args.functions {
        let function_flags = flags
            .iter()
            .filter(|flag| flag.fn_name == function)
            .collect::<Vec<&crate::registry::Flag>>();
        if function_flags.is_empty() {
            return Err(anyhow::anyhow!(format!("Function `{}` is not registered as a training function. Annotate a training function with #[heat(training)] to register it.", function)));
        } else if function_flags.len() > 1 {
            let function_strings = function_flags
                .iter()
                .map(|flag| {
                    format!(
                        "  {} {}::{}",
                        "-".custom_color(BURN_ORANGE),
                        flag.mod_path.bold(),
                        flag.fn_name.bold()
                    )
                })
                .collect::<Vec<String>>();
            return Err(anyhow::anyhow!(format!("Function `{}` is registered multiple times. Please write the entire module path of the desired function:\n{}", function.custom_color(BURN_ORANGE).bold(), function_strings.join("\n"))));
        }
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
