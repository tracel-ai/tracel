use crate::command_set::{HandlerMatchType, ShellCommandSetBase, UserAddedCommandMethod};
use crate::input_handler::{InputHandler, InputResult};
use clap::Command;

pub type ShellResult = anyhow::Result<ShellAction>;

pub enum ShellAction {
    Continue,
    UpdatePrompt(String),
    Exit,
}

pub trait ContextCommandFactory {
    fn get_command_mut(&mut self) -> &mut Command;
}

pub trait Handler<A, T> {
    fn handle(&self, args: A, context: &mut T) -> ShellResult;
}

/// A handler that takes a parser and a context and returns a ShellResult
/// This is a convenience implementation for closures
/// It allows you to pass a closure that takes a parser and a context and returns a ShellResult
/// as a handler to the Shell struct
impl<A, T, F> Handler<A, T> for F
where
    F: Fn(A, &mut T) -> ShellResult,
{
    fn handle(&self, args: A, context: &mut T) -> ShellResult {
        self(args, context)
    }
}

pub struct Shell<I: InputHandler> {
    app_name: String,
    prompt: String,
    command_sets: Vec<Box<dyn ShellCommandSetBase>>,
    cached_command_parser: Option<Command>,
    input_handler: I,
}

impl<I: InputHandler> Shell<I> {
    pub fn new(
        name: impl std::fmt::Display,
        prompt: impl std::fmt::Display,
        input_handler: I,
    ) -> Self {
        Self {
            app_name: name.to_string(),
            prompt: prompt.to_string(),
            command_sets: vec![],
            cached_command_parser: None,
            input_handler,
        }
    }

    pub fn register_command_set(&mut self, command_set: impl ShellCommandSetBase + 'static) {
        self.command_sets.push(Box::new(command_set));
    }

    pub fn run(&mut self) -> anyhow::Result<()> {
        loop {
            let input_res = self.input_handler.read(&self.prompt)?;
            match input_res {
                InputResult::Input(input) => {
                    let res = self.handle_input(&input);
                    match res {
                        Ok(shell_res) => match shell_res {
                            Ok(ShellAction::Continue) => continue,
                            Ok(ShellAction::UpdatePrompt(prompt)) => {
                                self.prompt = prompt;
                                continue;
                            }
                            Ok(ShellAction::Exit) => break,
                            Err(e) => println!("{}", e),
                        },
                        Err(e) => println!("{}", e),
                    }
                }
                InputResult::Interrupted => continue,
                InputResult::Eof => break,
            }
        }
        Ok(())
    }

    fn handle_input(&mut self, input: &str) -> Result<ShellResult, String> {
        // let args = shlex::split(input)
        //     .map(|args| P::try_parse_from(args))
        //     .ok_or("Invalid quoting")?
        //     // map err for clap to print the error message
        //     .map_err(|e| e.to_string())?;
        let raw_args = shlex::split(input).ok_or("Invalid quoting")?;

        // Here we are using the parser to parse the arguments before handing them to the handler
        // so that in case of a parsing error, we can early return and print the error message
        let command = self.get_command_mut();
        let args = command
            .try_get_matches_from_mut(raw_args)
            .map_err(|e| e.to_string())?;

        let res = self.dispatch(args);

        Ok(res)
    }

    fn get_command_mut(&mut self) -> &mut Command {
        if self.command_sets.iter().any(|set| set.is_dirty()) {
            self.cached_command_parser = None;
        }

        if self.cached_command_parser.is_none() {
            self.cached_command_parser = Some(self.create_clap());
        }

        self.cached_command_parser.as_mut().unwrap()
    }

    fn create_clap(&mut self) -> Command {
        let mut app = clap::Command::new(&self.app_name).multicall(true);

        for command_set in self.command_sets.iter_mut() {
            // let set_name = command_set.get_name();
            // if let Some(name) = set_name {
            //     app = app.subcommand(
            //         clap::Command::new(&name)
            //             .about(format!("{} commands", name))
            //             .subcommand_help_heading("Commands")
            //     );
            // }
            for method in command_set.get_commands() {
                app = match method {
                    UserAddedCommandMethod::Static(command) => app.subcommand(command.clone()),
                    UserAddedCommandMethod::Derived(augmenter) => augmenter(app),
                }
            }
        }
        app
    }

    fn dispatch(&mut self, args: clap::ArgMatches) -> ShellResult {
        if let Some((cmd_name, cmd_args)) = args.subcommand() {
            for command_set in &mut self.command_sets {
                if let Some(match_type) = command_set.has_command(cmd_name) {
                    return match match_type {
                        HandlerMatchType::Exact => {
                            command_set.handle_command(cmd_name, cmd_args.clone())
                        }
                        HandlerMatchType::Subcommand => {
                            command_set.handle_command(cmd_name, args.clone())
                        }
                    };
                }
            }
        }

        Ok(ShellAction::Continue)
    }
}
