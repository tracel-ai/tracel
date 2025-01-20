#![allow(warnings)]

use crate::child_cli::{ChildCliEvent, CliProcess, IpcCliProcess, ParentCliEvent, SyncInfo};
use clap::{FromArgMatches, Parser, Subcommand};
use ipc_channel::ipc::IpcSender;
use rustyline::ExternalPrinter;
use std::collections::HashMap;
use std::sync::Arc;

use std::io::BufReader;

use anyhow::Context;
use colored::Colorize;
use indicatif::HumanDuration;
use std::process::Stdio;
use std::time::{Duration, Instant};

use ctrlc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};

use crate::build_renderer;
use crate::command_set::{
    HandlerMatchType, ShellCommandSet, ShellCommandSetBase, UserAddedCommandMethod,
};
use crate::rustyline_handler::CustomEditorHandler;
use crate::shell::{self, Handler, ShellResult};
use crate::util::*;
use cargo_metadata::Message;
use notify::{RecursiveMode, Watcher};
use notify_debouncer_mini::new_debouncer;
use std::sync::mpsc::channel;
use tracel::heat::schemas::RegisteredHeatFunction;

#[derive(Clone, Debug, strum::Display, clap::ValueEnum)]
pub enum Backend {
    Wgpu,
    Cuda,
    Torch,
}

#[derive(Subcommand, Debug)]
pub enum ShellCommand {
    /// Print a message
    Echo {
        /// Message to print
        message: String,
    },
    /// Add two numbers
    Add {
        /// First number
        a: i32,
        /// Second number
        b: i32,
    },
    /// Select a computing backend
    Backend {
        /// Backend to use
        backend: Backend,
    },
    /// Recompiles and reloads the child process
    Reload,
    Login,

    /// Exit the REPL
    Exit,
}

/// Command-line parser for REPL commands
#[derive(Parser, Debug, Serialize, Deserialize)]
#[command(author, version, about, long_about = None)]
pub struct CliParser {
    #[arg(short, long, global = true)]
    verbose: bool,

    #[command(subcommand)]
    command: Option<CliCommand>,
}

/// Subcommands for the REPL
#[derive(Subcommand, Debug, Serialize, Deserialize)]
pub enum CliCommand {
    /// Print a message
    Echo {
        /// Message to print
        message: String,
    },
    /// Add two numbers
    Add {
        /// First number
        a: i32,
        /// Second number
        b: i32,
    },
}

fn is_remote() -> bool {
    std::env::var("HEATCLI_REMOTE")
        .map(|v| v == "1")
        .unwrap_or(false)
}

pub fn main() -> anyhow::Result<()> {
    if is_remote() {
        run_remote_cli()?;
    } else {
        let cli = CliParser::parse();

        if let Some(command) = cli.command {
            handle_cli_command(command, cli.verbose)?;
        } else {
            run_shell(cli)?;
        }
    }

    Ok(())
}

/// This struct holds the current state of the cli subprogram the shell is running
struct MetaCliState {
    registered_functions: Vec<RegisteredHeatFunction>,
}

impl MetaCliState {
    fn new() -> Self {
        Self {
            registered_functions: vec![],
        }
    }
}

struct InternalState {
    launch_args: CliParser,
    ctrlc: Arc<AtomicBool>,
    project_metadata: ProjectMetadata,
    current_binary: std::path::PathBuf,
    heat_dir: std::path::PathBuf,
    dirty: bool,
    reload_count: u32,
    process: Option<IpcCliProcess>,
    current_backend: Backend,
    cli_state: MetaCliState,
}

impl InternalState {
    pub fn new(
        launch_args: CliParser,
        project_metadata: ProjectMetadata,
        current_binary: std::path::PathBuf,
        heat_dir: std::path::PathBuf,
        editor: &mut CustomEditorHandler,
    ) -> Self {
        let ctrlc = Arc::new(AtomicBool::new(true));
        let ctrlc_inner = ctrlc.clone();
        ctrlc::set_handler(move || {
            ctrlc_inner.store(true, Ordering::SeqCst);
        })
        .expect("Failed to set Ctrl-C handler");

        let watcher_printer = editor
            .create_external_printer()
            .expect("Failed to create external printer");
        run_watcher_thread(watcher_printer);

        Self {
            launch_args,
            ctrlc,
            project_metadata,
            current_binary,
            heat_dir,
            dirty: false,
            reload_count: 0,
            process: None,
            current_backend: Backend::Wgpu,
            cli_state: MetaCliState::new(),
        }
    }

    pub fn collect_new_binary(&mut self, built_exe_path: std::path::PathBuf) -> anyhow::Result<()> {
        // Delete old binary if it exists
        if self.current_binary.exists() {
            std::fs::remove_file(&self.current_binary).context("Failed to remove old binary")?;
        }

        println!("Found built binary at {:?}", built_exe_path);
        let build_id = uuid::Uuid::new_v4().simple();
        // relocate the new built file to the heat dir
        let new_bin_name = format!(
            "{}_{}{}",
            self.project_metadata.bin_name,
            build_id,
            std::env::consts::EXE_SUFFIX
        );
        let new_bin_path =
            try_relocate_to_dir(&built_exe_path, &self.heat_dir, &new_bin_name, true)
                .context("Failed to relocate new built file")?;

        println!("Copied new binary");
        self.current_binary = new_bin_path;
        self.dirty = true;
        self.reload_count += 1;

        Ok(())
    }

    pub fn execute_command(&mut self, command: CliCommand) -> anyhow::Result<()> {
        self.process = Some(
            IpcCliProcess::new(&self.current_binary).context("Failed to start child process")?,
        );

        let parser = CliParser {
            verbose: self.launch_args.verbose,
            command: Some(command),
        };

        self.process
            .as_mut()
            .unwrap()
            .send_event(ParentCliEvent::Input(parser))
            .context("Failed to send command")?;

        self.process.as_mut().unwrap().wait()?;

        Ok(())
    }

    pub fn select_backend(&mut self, backend: Backend) {
        self.current_backend = backend;
    }

    pub fn sync(&mut self) -> anyhow::Result<()> {
        self.process.replace(
            IpcCliProcess::new(&self.current_binary).context("Failed to start child process")?,
        );

        self.process
            .as_mut()
            .unwrap()
            .send_event(ParentCliEvent::Sync)
            .context("Failed to send sync command")?;

        let sync_info = self
            .process
            .as_mut()
            .unwrap()
            .receive_event()
            .context("Failed to receive sync response")?;

        match sync_info {
            ChildCliEvent::SyncResponse(sync_info) => {
                self.cli_state.registered_functions = sync_info.functions;
            }
            _ => (),
        }

        Ok(())
    }

    pub fn build_command(&self) -> std::process::Command {
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
        let mut cmd = std::process::Command::new(cargo);
        cmd.arg("build")
            .arg("--message-format=json-render-diagnostics")
            .args(["--bin", &self.project_metadata.bin_name]);
        cmd
    }

    pub fn get_prompt(&self) -> String {
        format!("{}> ", &self.project_metadata.bin_name.red().bold())
    }

    pub fn dirty(&self) -> bool {
        self.dirty
    }

    pub fn clear_dirty(&mut self) {
        self.dirty = false;
    }
}

fn try_locate_heat_dir() -> anyhow::Result<std::path::PathBuf> {
    let manifest_dir = locate_project_dir().context("Failed to locate project directory")?;
    let heat_dir = manifest_dir.join(".heat");
    if !heat_dir.exists() {
        std::fs::create_dir_all(&heat_dir).context("Failed to create heat directory")?;
    }

    Ok(heat_dir)
}

type DynamicCommandRegistry = HashMap<String, Box<dyn Handler<clap::ArgMatches, InternalState>>>;

pub struct InternalShellCommandSet {
    dynamic_commands_handlers: DynamicCommandRegistry,
    cached_dynamic_commands: Vec<clap::Command>,
    inner_shell_command_set: ShellCommandSet<InternalState>,
}

impl InternalShellCommandSet {
    pub fn new(inner_shell_command_set: ShellCommandSet<InternalState>) -> Self {
        Self {
            dynamic_commands_handlers: DynamicCommandRegistry::new(),
            cached_dynamic_commands: vec![],
            inner_shell_command_set,
        }
    }
}

impl ShellCommandSetBase for InternalShellCommandSet {
    fn get_name(&self) -> Option<String> {
        self.inner_shell_command_set.name.clone()
    }
    fn is_dirty(&self) -> bool {
        self.inner_shell_command_set.state.dirty()
    }

    fn get_commands(&mut self) -> Vec<UserAddedCommandMethod> {
        let mut train_command = clap::Command::new("train")
            .about("Execute a training function")
            .subcommand_required(true)
            .args(&[
                clap::arg!(-c --config <FILE> "Path to the training configuration file")
                    .required(true),
            ]);

        {
            let functions = &self
                .inner_shell_command_set
                .state
                .cli_state
                .registered_functions;
            for function in functions {
                if function.proc_type == "train" {
                    train_command = train_command.subcommand(
                        clap::Command::new(&function.fn_name)
                            .about(format!("Run training function {}", function.mod_path)),
                    );
                }
            }
        }

        self.dynamic_commands_handlers.insert(
            "train".to_string(),
            Box::new(
                |args: clap::ArgMatches, context: &mut InternalState| -> ShellResult {
                    let (subcommand, subcommand_args) =
                        args.subcommand().expect("Failed to get subcommand");
                    let subcommand = subcommand.to_string();
                    let subcommand_args = subcommand_args.clone();

                    let functions = &context.cli_state.registered_functions;
                    let function = functions.iter().find(|f| f.fn_name == subcommand);
                    if let Some(function) = function {
                        let command = CliCommand::Echo {
                            message: format!(
                                "Running training function {} on {}",
                                function.fn_name, context.current_backend
                            ),
                        };
                        context
                            .execute_command(command)
                            .context("Failed to execute command")?;
                    }

                    Ok(shell::ShellAction::Continue)
                },
            ),
        );

        let mut infer_command = clap::Command::new("infer")
            .about("Execute an inference function")
            .subcommand_required(true)
            .args(&[
                clap::arg!(-c --config <FILE> "Path to the inference configuration file")
                    .required(true),
            ]);

        {
            let functions = &self
                .inner_shell_command_set
                .state
                .cli_state
                .registered_functions;
            for function in functions {
                if function.proc_type == "infer" {
                    infer_command = infer_command.subcommand(
                        clap::Command::new(&function.fn_name)
                            .about(format!("Run inference function {}", function.mod_path)),
                    );
                }
            }
        }

        self.dynamic_commands_handlers.insert(
            "infer".to_string(),
            Box::new(
                |args: clap::ArgMatches, context: &mut InternalState| -> ShellResult {
                    let (subcommand, subcommand_args) =
                        args.subcommand().expect("Failed to get subcommand");
                    let subcommand = subcommand.to_string();
                    let subcommand_args = subcommand_args.clone();

                    let functions = &context.cli_state.registered_functions;
                    let function = functions.iter().find(|f| f.fn_name == subcommand);
                    if let Some(function) = function {
                        let command = CliCommand::Echo {
                            message: format!(
                                "Running inference function {} on {}",
                                function.fn_name, context.current_backend
                            ),
                        };
                        context
                            .execute_command(command)
                            .context("Failed to execute command")?;
                    }

                    Ok(shell::ShellAction::Continue)
                },
            ),
        );

        let base_commands = self.inner_shell_command_set.get_commands();

        let mut commands = vec![
            UserAddedCommandMethod::Static(train_command),
            UserAddedCommandMethod::Static(infer_command),
        ];
        commands.extend(base_commands);

        commands
    }

    fn has_command(&self, command: &str) -> Option<HandlerMatchType> {
        if let Some(handler) = self.inner_shell_command_set.handlers.get(command) {
            return Some(handler.match_type.clone());
        }
        if self.dynamic_commands_handlers.get(command).is_some() {
            return Some(HandlerMatchType::Exact);
        }
        None
    }

    fn handle_command(&mut self, command: &str, args: clap::ArgMatches) -> ShellResult {
        // handle inner_shell_command_set commands first
        if self.inner_shell_command_set.has_command(command).is_some() {
            return self.inner_shell_command_set.handle_command(command, args);
        }
        if let Some(handler) = self.dynamic_commands_handlers.get(command) {
            return handler.handle(args, &mut self.inner_shell_command_set.state);
        }

        Ok(shell::ShellAction::Continue)
    }
}

pub fn create_internal_shell_context(
    args: CliParser,
    input_handler: &mut CustomEditorHandler,
) -> anyhow::Result<InternalState> {
    // The binary name must have been set prior to running the shell
    // This is because the binary name is used to locate the cargo build artifacts
    assert!(
        crate::__internals::get_binary_name().is_some(),
        "Binary name must be set before running the shell"
    );

    // collect the project metadata
    let project_metadata = try_get_project_metadata(
        std::env::var("CARGO_PKG_NAME")?,
        crate::__internals::get_binary_name().unwrap().to_string(),
    )
    .context("Failed to get project metadata")?;

    let heat_dir = try_locate_heat_dir().context("Failed to locate heat directory")?;
    println!("{}", heat_dir.display());

    // this is the new location of the currently running executable
    let current_binary = try_relocate_cargo_build_bin_to_dir(
        &project_metadata.target_directory,
        &project_metadata.bin_name,
        &heat_dir,
    )
    .context("Failed to relocate cargo build binaries")?;
    println!("Self-relocated program binaries");

    let mut ctx = InternalState::new(
        args,
        project_metadata,
        current_binary,
        heat_dir,
        input_handler,
    );
    ctx.sync()
        .context("Failed to do first sync with child process")?;

    Ok(ctx)
}

pub fn run_shell(args: CliParser) -> anyhow::Result<()> {
    let mut input_handler = CustomEditorHandler::new()?;
    let ctx = create_internal_shell_context(args, &mut input_handler)?;
    let initial_prompt = ctx.get_prompt();

    let internal_command_set = ShellCommandSet::with_name("internals".to_string(), ctx)
        .register_subcommand_parser(handle_shell_command);
    let internal_shell_command_set = InternalShellCommandSet::new(internal_command_set);

    let mut shell = shell::Shell::new(
        std::env::var("CARGO_PKG_NAME")?,
        initial_prompt,
        input_handler,
    );

    shell.register_command_set(internal_shell_command_set);

    shell.run()?;

    Ok(())
}

fn handle_shell_command(command: ShellCommand, context: &mut InternalState) -> ShellResult {
    match command {
        ShellCommand::Add { a, b } => context
            .execute_command(CliCommand::Add { a, b })
            .context("Failed to execute command")?,
        ShellCommand::Echo { message } => context
            .execute_command(CliCommand::Echo { message })
            .context("Failed to execute command")?,
        ShellCommand::Backend { backend } => {
            println!("Selected backend {}", backend);
            context.select_backend(backend);
        }
        ShellCommand::Exit => return Ok(shell::ShellAction::Exit),
        ShellCommand::Reload => {
            println!("Reloading...");

            context.ctrlc.store(false, Ordering::SeqCst);

            let started = Instant::now();

            // Recompile the child process
            let mut p = context
                .build_command()
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .context("Failed to recompile")?;

            let mut artifact_paths = vec![];
            let mut success = false;
            let mut build_renderer = build_renderer::CargoBuildRenderer::new();

            let p_stdout = p.stdout.take().unwrap();
            let mut buf_reader_lines_out = Message::parse_stream(BufReader::new(p_stdout));
            while !context.ctrlc.load(Ordering::SeqCst) {
                let next = buf_reader_lines_out.next();
                // // animate the spinner
                if let Some(Ok(message)) = next {
                    match &message {
                        Message::CompilerArtifact(msg) => {
                            if let Some(executable) = &msg.executable {
                                artifact_paths.push(executable.clone());
                            }
                        }
                        Message::BuildFinished(msg) => {
                            success = msg.success;
                        }
                        _ => (),
                    }
                    build_renderer.render(message);
                }
                build_renderer.tick();

                if let Ok(Some(_)) = p.try_wait() {
                    break;
                }
            }

            let cancelled = context.ctrlc.load(Ordering::SeqCst);

            // Wait for the process to finish
            let _ = p.wait();

            build_renderer.finish();

            if cancelled {
                println!("Compilation aborted, reload cancelled");
            } else {
                if success {
                    println!("Done in {}", HumanDuration(started.elapsed()));

                    let artifact = artifact_paths
                        .into_iter()
                        .find(|path| {
                            path.file_stem()
                                .map(|s| s == context.project_metadata.bin_name)
                                .unwrap_or(false)
                        })
                        .expect("Failed to find built executable");
                    let built_exe_path = artifact.into_std_path_buf();
                    built_exe_path.try_exists().with_context(|| {
                        format!(
                            "Failed to find built executable at {}",
                            built_exe_path.display()
                        )
                    })?;
                    context
                        .collect_new_binary(built_exe_path)
                        .context("Failed to collect new binary")?;
                    context.sync().context("Failed to sync")?;
                    println!(
                        "Process {} reloaded in {}",
                        context.project_metadata.bin_name.red().bold(),
                        HumanDuration(started.elapsed())
                    );
                } else {
                    println!("Build failed");
                }
            }
            // reset the ctrl-c flag for the next iteration
            context.ctrlc.store(false, Ordering::SeqCst);
        }
        ShellCommand::Login => anyhow::bail!("Login command not implemented"),
    }

    Ok(shell::ShellAction::Continue)
}

fn run_watcher_thread(mut printer: impl ExternalPrinter + Send + 'static) {
    let (tx, rx) = channel();
    let mut debouncer = new_debouncer(Duration::from_secs(1), tx).unwrap();

    let watch_path = std::env::current_dir().expect("Failed to get curent directory");
    debouncer
        .watcher()
        .watch(&watch_path, RecursiveMode::Recursive)
        .expect("Failed to watch source files");

    // watcher thread
    std::thread::spawn(move || {
        let _keep_alive = debouncer;
        loop {
            match rx.recv() {
                Ok(Ok(events)) => {
                    for event in events {
                        // Handle only `.rs` file modifications
                        let rs_file_modified =
                            event.path.extension().map_or(false, |ext| ext == "rs");

                        if !rs_file_modified {
                            continue;
                        }

                        let relative_path =
                            event.path.strip_prefix(&watch_path).unwrap_or(&event.path);
                        printer
                            .print(format!("File {:?} modified", relative_path))
                            .expect("TODO: panic message");
                    }
                }
                Err(err) => {
                    eprintln!("Watcher error1: {:?}", err);
                    break;
                }
                Ok(Err(err)) => {
                    eprintln!("Watcher error2: {:?}", err);
                    break;
                }
            }
        }
    });
}

fn run_remote_cli() -> anyhow::Result<()> {
    // Parse cli arguments

    let cmd_channel_id = std::env::var("HEATCLI_PARENT_CHANNEL_ID")?;
    let resp_channel_id = std::env::var("HEATCLI_CHILD_CHANNEL_ID")?;

    let (cmd_sender, cmd_receiver) = ipc_channel::ipc::channel::<ParentCliEvent>()?;
    let (resp_sender, resp_receiver) = ipc_channel::ipc::channel::<ChildCliEvent>()?;

    let cmd_bootstrap = IpcSender::connect(cmd_channel_id)?;
    cmd_bootstrap.send(cmd_sender)?;

    let resp_bootstrap = IpcSender::connect(resp_channel_id)?;
    resp_bootstrap.send(resp_receiver)?;

    let event = cmd_receiver.recv()?;

    match event {
        ParentCliEvent::Input(cli) => match cli.command {
            Some(command) => {
                handle_cli_command(command, cli.verbose)?;
            }
            None => {
                println!("No command received");
            }
        },
        ParentCliEvent::Sync => {
            let current_dir = std::env::current_dir()?;
            let functions = vec![
                RegisteredHeatFunction {
                    mod_path: "path::to::test".to_string(),
                    code: "".to_string(),
                    fn_name: "test".to_string(),
                    proc_type: "train".to_string(),
                },
                RegisteredHeatFunction {
                    mod_path: "path::to::test2".to_string(),
                    code: "".to_string(),
                    fn_name: "test2".to_string(),
                    proc_type: "train".to_string(),
                },
                RegisteredHeatFunction {
                    mod_path: "hell::satan::blasphem".to_string(),
                    code: "".to_string(),
                    fn_name: "blasphem".to_string(),
                    proc_type: "train".to_string(),
                },
                RegisteredHeatFunction {
                    mod_path: "do::not::touch".to_string(),
                    code: "balbalblalballbalba".to_string(),
                    fn_name: "touch".to_string(),
                    proc_type: "train".to_string(),
                },
                RegisteredHeatFunction {
                    mod_path: "path::to::nothingburger".to_string(),
                    code: "".to_string(),
                    fn_name: "nothingburger".to_string(),
                    proc_type: "infer".to_string(),
                },
            ];

            let sync_info = SyncInfo {
                current_dir,
                functions,
            };

            resp_sender.send(ChildCliEvent::SyncResponse(sync_info))?;
        }
    }

    Ok(())
}

fn handle_cli_command(command: CliCommand, verbose: bool) -> anyhow::Result<()> {
    match command {
        CliCommand::Echo { message } => {
            println!("{}", message);
        }
        CliCommand::Add { a, b } => {
            if verbose {
                println!("Adding {} and {}", a, b);
            }
            println!("{}", a + b);
        }
    }

    Ok(())
}
