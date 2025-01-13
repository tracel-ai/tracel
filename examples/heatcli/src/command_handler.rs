use clap::Command;
use crate::command_set::{HandlerMatchType, ShellCommandSetBase, UserAddedCommandMethod};
use crate::shell::{ShellAction, ShellResult};

pub struct CommandSetRegistry {
    command_sets: Vec<Box<dyn ShellCommandSetBase>>,
    cached_command_parser: Option<Command>,
}

impl CommandSetRegistry {
    pub fn new(
    ) -> Self {
        Self {
            command_sets: vec![],
            cached_command_parser: None,
        }
    }

    pub fn add_command_set(&mut self, command_set: impl ShellCommandSetBase + 'static) {
        self.command_sets.push(Box::new(command_set));
    }

    fn create_clap(&mut self) -> Command {
        let mut app = clap::Command::default()
            .multicall(true);

        for command_set in self.command_sets.iter_mut() {
            let set_name = command_set.get_name();
            if let Some(name) = set_name {
                app = app.subcommand(
                    clap::Command::new(&name)
                        .about(format!("{} commands", name))
                        .subcommand_help_heading("Commands")
                );
            }
            for method in command_set.get_commands() {
                app = match method {
                    UserAddedCommandMethod::Static(command) => app.subcommand(command.clone()),
                    UserAddedCommandMethod::Derived(augmenter) => augmenter(app),
                }
            }
        }
        app
    }

    pub fn get_command_mut(&mut self) -> &mut Command {
        if self.command_sets.iter().any(|set| set.is_dirty()) {
            self.cached_command_parser = None;
        }

        if self.cached_command_parser.is_none() {
            self.cached_command_parser = Some(self.create_clap());
        }

        self.cached_command_parser.as_mut().unwrap()
    }

    pub fn dispatch(&mut self, args: clap::ArgMatches) -> ShellResult {
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
                    }
                }
            }
        }

        Ok(ShellAction::Continue)
    }
    
}