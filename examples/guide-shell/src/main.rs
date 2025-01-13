use clap::{ArgMatches, Command, Parser, Subcommand};

// /// Command-line parser for REPL commands
// #[derive(Parser, Debug)]
// #[command(author, version, about, long_about = None)]
// pub struct CliParser {
//     #[command(subcommand)]
//     command: CliCommand,
// }
//
//
// /// Subcommands for the REPL
// #[derive(Subcommand, Debug)]
// pub enum CliCommand {
//     /// Print a message
//     Echo2 {
//         /// Message to print
//         message: String,
//     },
//     /// Add two numbers
//     Add2 {
//         /// First number
//         a: i32,
//         /// Second number
//         b: i32,
//     },
//     Say2 {
//         /// Say hello or goodbye
//         #[command(subcommand)]
//         subcommand: CliSubcommand,
//     },
// }
//
// #[derive(Subcommand, Debug)]
// pub enum CliSubcommand {
//     Hello2 {
//         /// Name to say hello to
//         name: String,
//     },
//     Bye2,
// }
//
// pub struct MyShellState {
//     hello: String,
// }


/* generated */
fn main() -> Result<(), Box<dyn std::error::Error>> {
    heatcli::__internals::set_binary_name(heatcli::capture_bin_name!());
    heatcli::main()?;

    Ok(())
}

/* generated */

// /* #[heatcli::main] */
// fn loader() -> heatcli::ShellBuilderResult<MyShellState> {
//     let state = MyShellState {
//         hello: "".to_string(),
//     };
//
//     let hello_cmd = heatcli::clap::Command::new("hello")
//         .about("Say hello")
//         .arg(heatcli::clap::Arg::new("name")
//             .help("The name to say hello to")
//             .required(true)
//             .index(1))
//         .arg(heatcli::clap::Arg::new("loud")
//             .help("Whether to say hello loudly")
//             .short('l')
//             .long("loud"));
//
//     let bye_cmd = heatcli::clap::Command::new("bye")
//         .about("Say goodbye");
//
//     let nested_cmd = heatcli::clap::Command::new("say")
//         .about("A nested command, to say hello or goodbye")
//         .subcommand_required(true)
//         .subcommand(hello_cmd)
//         .subcommand(bye_cmd);
//
//     let test_cmd = heatcli::clap::Command::new("test")
//         .about("A test command");
//
//     let cmd = CliCommand::augment_subcommands(test_cmd);
//
//
//
//     let builder = heatcli::ShellCommandSet::with_state(state)
//         .add_command(nested_cmd, |args: ArgMatches, _state: &mut MyShellState| -> heatcli::shell::ShellResult {
//             let (cmd_name, cmd_matches): (&str, &ArgMatches) = args.subcommand().unwrap();
//             match cmd_name {
//                 "hello" => {
//                     let name = cmd_matches.get_one::<String>("name").unwrap();
//                     let loud = cmd_matches.get_one::<String>("loud");
//                     if let Some(_) = loud {
//                         println!("HELLO, {}!", name);
//                     } else {
//                         println!("Hello, {}!", name);
//                     }
//                 }
//                 "bye" => {
//                     println!("Goodbye!");
//                 }
//                 _ => {
//                     println!("Unknown command: {}", cmd_name);
//                 }
//             }
//             Ok(heatcli::shell::ShellAction::Continue)
//         })
//         .add_command(cmd, |args: ArgMatches, _state: &mut MyShellState| -> heatcli::shell::ShellResult {
//             let (cmd_name, cmd_matches): (&str, &ArgMatches) = args.subcommand().unwrap();
//             println!("Running test command: {} \n{:?}", cmd_name, cmd_matches);
//             Ok(heatcli::shell::ShellAction::Continue)
//         })
//         .register_subcommand_parser(handle_subcommand)
//         .register_parser(handle_parser);
//     Ok(builder)
// }
//
// fn handle_parser(command: CliParser, state: &mut MyShellState) -> heatcli::shell::ShellResult {
//     println!("{:?}", command);
//     Ok(heatcli::shell::ShellAction::Continue)
// }
//
// fn handle_subcommand(command: CliCommand, state: &mut MyShellState) -> heatcli::shell::ShellResult {
//     match command {
//         CliCommand::Echo2 { message } => {
//             println!("{}", message);
//         }
//         CliCommand::Add2 { a, b } => {
//             println!("{}", a + b);
//         }
//         CliCommand::Say2 { subcommand } => {
//             match subcommand {
//                 CliSubcommand::Hello2 {
//                     name,
//                 } => {
//                     println!("Hello, {}!", name);
//                 }
//                 CliSubcommand::Bye2 => {
//                     println!("Goodbye!");
//                 }
//             }
//         }
//     }
//
//     Ok(heatcli::shell::ShellAction::Continue)
// }