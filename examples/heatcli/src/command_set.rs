use std::collections::HashMap;
use std::sync::Arc;
use clap::{Parser, Subcommand};
use crate::shell;
use crate::shell::ShellResult;

#[derive(Clone)]
pub enum UserAddedCommandMethod {
    Static(clap::Command),
    Derived(Arc<dyn Fn(clap::Command) -> clap::Command>),
}

impl std::fmt::Debug for UserAddedCommandMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UserAddedCommandMethod::Static(cmd) => write!(f, "Static({:?})", cmd),
            UserAddedCommandMethod::Derived(_) => write!(f, "Derived"),
        }
    }
}

#[derive(Clone)]
pub enum HandlerMatchType {
    Exact,
    Subcommand,
}


pub struct CommandSetHandler<T> {
    pub match_type: HandlerMatchType,
    pub handler: Arc<dyn shell::Handler<clap::ArgMatches, T>>,
}

pub struct ShellCommandSet<T> {
    pub name: Option<String>,
    pub state: T,
    pub commands: Vec<UserAddedCommandMethod>,
    pub handlers: HashMap<String, CommandSetHandler<T>>,
}

impl<T> ShellCommandSet<T> {

    pub fn with_state(state: T) -> Self {
        Self {
            name: None,
            state,
            commands: Default::default(),
            handlers: Default::default(),
        }
    }

    pub fn with_name(name: String, state: T) -> Self {
        Self {
            name: Some(name),
            state,
            commands: Default::default(),
            handlers: Default::default(),
        }
    }

    pub fn add_command(mut self, command: clap::Command, handler: impl shell::Handler<clap::ArgMatches, T> + 'static) -> Self {
        let cmd_name = command.get_name().to_string();
        self.commands.push(UserAddedCommandMethod::Static(command));
        self.handlers.insert(cmd_name, CommandSetHandler {
            match_type: HandlerMatchType::Exact,
            handler: Arc::new(handler),
        });
        self
    }

    pub fn register_parser<P: Parser>(mut self, handler: impl shell::Handler<P, T> + 'static) -> Self {
        let wrapped_handler = move |args: clap::ArgMatches, state: &mut T| -> ShellResult {
            let args = P::from_arg_matches(&args).map_err(anyhow::Error::from)?;
            handler.handle(args, state)
        };

        let command = P::command_for_update();
        let cmd_name = command.get_name().to_string();
        self.commands.push(UserAddedCommandMethod::Static(command));
        self.handlers.insert(cmd_name, CommandSetHandler {
            match_type: HandlerMatchType::Exact,
            handler: Arc::new(wrapped_handler),
        });
        self
    }

    pub fn register_subcommand_parser<S: Subcommand>(mut self, handler: impl shell::Handler<S, T> + 'static) -> Self {
        let augmenter = |c| S::augment_subcommands(c);
        self.commands.push(UserAddedCommandMethod::Derived(Arc::new(augmenter)));


        let wrapped_handler = move |args: clap::ArgMatches, state: &mut T| -> ShellResult {
            let args = S::from_arg_matches(&args).map_err(anyhow::Error::from)?;
            handler.handle(args, state)
        };
        let handler = Arc::new(wrapped_handler);

        // get all subcommand names
        let dummy_command = augmenter(clap::Command::new("dummy"));
        let subcommands = dummy_command.get_subcommands().map(|c| c.get_name().to_string());

        for subcommand in subcommands {
            println!("Registering subcommand: {} for handler", subcommand);
            self.handlers.insert(subcommand, CommandSetHandler {
                match_type: HandlerMatchType::Subcommand,
                handler: handler.clone(),
            });
        }
        self
    }

    // pub(crate) fn build(self, ctx: RemoteShell) -> ShellState<T> {
    //     let user_added_commands = self.command_methods;
    //     let handlers = self.subcommand_handlers.into_iter().collect();
    //     ShellState::new(ctx, self.state, user_added_commands, handlers)
    // }
}

pub trait ShellCommandSetBase: std::any::Any {
    fn get_name(&self) -> Option<String>;
    fn is_dirty(&self) -> bool;
    fn get_commands(&mut self) -> Vec<UserAddedCommandMethod>;
    fn has_command(&self, command: &str) -> Option<HandlerMatchType>;
    fn handle_command(&mut self, command: &str, args: clap::ArgMatches) -> ShellResult;
}

impl<T: 'static> ShellCommandSetBase for ShellCommandSet<T> {
    fn get_name(&self) -> Option<String> {
        self.name.clone()
    }
    fn is_dirty(&self) -> bool {
        false
    }
    fn get_commands(&mut self) -> Vec<UserAddedCommandMethod> {
        self.commands.clone()
    }

    fn has_command(&self, command: &str) -> Option<HandlerMatchType> {
        self.handlers.get(command).map(|h| h.match_type.clone())
    }

    fn handle_command(&mut self, command: &str, args: clap::ArgMatches) -> ShellResult {
        if let Some(handler) = self.handlers.get(command) {
            return handler.handler.handle(args, &mut self.state);
        }

        Err(anyhow::anyhow!("Cmd not found"))
    }
}