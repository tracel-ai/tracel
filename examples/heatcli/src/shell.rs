use std::collections::HashMap;
use clap::{ArgMatches, Command, Error, FromArgMatches, Parser};
use crate::app::CommandRegistry;
use crate::command_handler::CommandSetRegistry;
use crate::command_set::HandlerMatchType;
use crate::input_handler::{InputHandler, InputResult};

pub type ShellResult = anyhow::Result<ShellAction>;

pub enum ShellAction {
    Continue,
    UpdatePrompt(String),
    Exit
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
where F: Fn(A, &mut T) -> ShellResult
{
    fn handle(&self, args: A, context: &mut T) -> ShellResult {
        self(args, context)
    }
}

pub struct Shell<I: InputHandler> {
    pub prompt: String,
    pub command_registry: CommandSetRegistry,
    // pub handler: H,
    pub input_handler: I,
}

impl<I: InputHandler> Shell<I> {
    pub fn new(prompt: impl std::fmt::Display, command_registry: CommandSetRegistry, input_handler: I) -> Self {
        Self {
            prompt: prompt.to_string(),
            command_registry,
            input_handler,
        }
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
                            },
                            Ok(ShellAction::Exit) => break,
                            Err(e) => println!("{}", e)
                        },
                        Err(e) => println!("{}", e)
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
        let command = self.command_registry.get_command_mut();
        let args = command.try_get_matches_from_mut(raw_args).map_err(|e| e.to_string())?;

        let res = self.command_registry.dispatch(args);

        Ok(res)
    }
}