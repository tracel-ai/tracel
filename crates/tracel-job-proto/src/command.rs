//! The command line of a program's jobs, built from their definitions.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use clap::builder::{
    PathBufValueParser, PossibleValue, PossibleValuesParser, TypedValueParser, ValueParser,
};
use clap::parser::ValueSource;
use clap::{Arg, ArgMatches, Command, ValueHint, value_parser};
use clap_complete::Shell;
use serde_json::{Map, Value};

use crate::JobDefinition;
use crate::flags::{Flag, Kind, flags, scalar_text};

/// The id of a job's input argument, which no field's flag has: field ids have no `:`.
const INPUT: &str = "tracel:input";
/// The id of a job's `--config` argument.
const CONFIG: &str = "tracel:config";
/// The id of the program's `--completions` argument.
const COMPLETIONS: &str = "tracel:completions";

/// The command line of the program `name` that runs the jobs `definitions` describes:
/// `<name> <job> [<input-json>] [--<field> <value>...]`, with one subcommand per job, as
/// [`job_command`] builds it.
///
/// `<name> --completions <SHELL>` asks for the program's completion script, which
/// [`completions`] reads and [`clap_complete::generate`] writes from this command.
///
/// The command depends only on `name` and the definitions, so a program that reads them from a
/// [`DefinitionsFile`](crate::DefinitionsFile) builds the same command line as the program that
/// wrote it, but for `--version`, which a program adds when it gives its version.
pub fn command<'a>(
    name: impl Into<String>,
    definitions: impl IntoIterator<Item = &'a JobDefinition>,
) -> Command {
    Command::new(name.into())
        .subcommand_value_name("JOB")
        .subcommand_help_heading("Jobs")
        .disable_help_subcommand(true)
        .args_conflicts_with_subcommands(true)
        .arg(
            Arg::new(COMPLETIONS)
                .long("completions")
                .value_name("SHELL")
                .value_parser(value_parser!(Shell))
                .help("Print the completion script for SHELL"),
        )
        .subcommands(definitions.into_iter().map(job_command))
}

/// The command line of the job `definition` describes, named after it, which
/// [`job_input`] reads the job's input from.
///
/// It takes three arguments, each merged onto the one before, so a later one wins:
///
/// 1. `-c, --config <FILE>`: a file holding a JSON document.
/// 2. `[INPUT]`: a JSON document, which launchers pass.
/// 3. One flag per field of the input, which sets that field.
///
/// With an input schema that describes an object, each field the schema gives a type is a flag
/// of that type: a boolean, an integer, a number, a string, or one of the values the schema
/// lists. Its description is the flag's help, and a field the schema requires is a required flag
/// unless the example input gives it or `--config` or `[INPUT]` is given. Without one, each
/// field of the example input is a flag typed by its value. A field that is an array, a map, a
/// nullable object or of no single type, or whose example value is `null`, takes a JSON literal.
/// The example input gives each flag's default, which the job fills in from its default input.
///
/// A nested field's flag joins the keys with dots, and writes `_` as `-`: the field
/// `weight_decay` of `optimizer` is `--optimizer.weight-decay`. A boolean flag given no value is
/// `true`. `--help`, `--version` and `--config` belong to the runner, so a field named `help`,
/// `version` or `config` has no flag; neither does one whose key is not made of letters, digits,
/// `_` and `-`, or whose flag name an earlier field took. Such a field is set through `--config`
/// or `[INPUT]`.
pub fn job_command(definition: &JobDefinition) -> Command {
    let command = Command::new(definition.name.clone())
        .args_override_self(true)
        .args(flags(definition).iter().map(flag_arg))
        .arg(
            Arg::new(INPUT)
                .value_name("INPUT")
                .value_parser(|input: &str| Kind::Json.parse(input))
                .allow_negative_numbers(true)
                .help("JSON merged after --config and before the flags"),
        )
        .arg(
            Arg::new(CONFIG)
                .short('c')
                .long("config")
                .value_name("FILE")
                .value_hint(ValueHint::FilePath)
                .value_parser(PathBufValueParser::new())
                .help("JSON merged before the input and the flags"),
        );
    match &definition.description {
        Some(description) => command.about(description.clone()),
        None => command,
    }
}

/// The input the arguments of a job's command line give, as [`job_command`] built it, for the
/// job to merge onto its default input. `Value::Null`, no input, when none is given.
///
/// The `--config` file, the JSON input and the flags are overlaid in that order. Overlaying
/// follows JSON merge patch (RFC 7386), except that a `null` field is kept rather than applied:
/// the fields of an object replace those of the object below or are overlaid onto them, and any
/// other value replaces what is below. A `null` field still removes that field from the job's
/// default input, which the input is merged onto. One of them alone is the input as given.
///
/// `read_config` reads the `--config` file, such as with `std::fs::read_to_string`, when the
/// arguments name one. Fails when it cannot be read or is not JSON.
pub fn job_input(
    matches: &ArgMatches,
    read_config: impl FnOnce(&Path) -> io::Result<String>,
) -> Result<Value, ConfigError> {
    let mut fields = Value::Null;
    for id in matches.ids().map(|id| id.as_str()) {
        if id == INPUT || id == CONFIG || matches.value_source(id) != Some(ValueSource::CommandLine)
        {
            continue;
        }
        if let Ok(Some(value)) = matches.try_get_one::<Value>(id) {
            let path: Vec<&str> = id.split('.').collect();
            set(&mut fields, &path, value.clone());
        }
    }
    let config = match matches.try_get_one::<PathBuf>(CONFIG).ok().flatten() {
        Some(path) => Some(config(path, read_config)?),
        None => None,
    };
    let given = matches.try_get_one::<Value>(INPUT).ok().flatten().cloned();
    let fields = Some(fields).filter(|fields| !fields.is_null());
    Ok([config, given, fields]
        .into_iter()
        .flatten()
        .reduce(|mut input, layer| {
            overlay(&mut input, layer);
            input
        })
        .unwrap_or(Value::Null))
}

/// The JSON document of the `--config` file at `path`, read with `read`.
fn config(
    path: &Path,
    read: impl FnOnce(&Path) -> io::Result<String>,
) -> Result<Value, ConfigError> {
    let unreadable = |source| ConfigError::Unreadable {
        path: path.to_path_buf(),
        source,
    };
    let contents = read(path).map_err(unreadable)?;
    serde_json::from_str(&contents).map_err(|source| ConfigError::NotJson {
        path: path.to_path_buf(),
        source,
    })
}

/// Why the `--config` file of a job's command line gives no input.
#[derive(Debug)]
pub enum ConfigError {
    /// The file cannot be read.
    Unreadable {
        /// The path the command line gives.
        path: PathBuf,
        /// Why it cannot be read.
        source: io::Error,
    },
    /// The file is not a JSON document.
    NotJson {
        /// The path the command line gives.
        path: PathBuf,
        /// Why it is not JSON.
        source: serde_json::Error,
    },
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreadable { path, source } => {
                write!(
                    f,
                    "cannot read the --config file {}: {source}",
                    path.display()
                )
            }
            Self::NotJson { path, source } => {
                write!(
                    f,
                    "the --config file {} is not JSON: {source}",
                    path.display()
                )
            }
        }
    }
}

impl std::error::Error for ConfigError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Unreadable { source, .. } => Some(source),
            Self::NotJson { source, .. } => Some(source),
        }
    }
}

/// The shell whose completion script the arguments of a [`command`] ask for with
/// `--completions <SHELL>`.
pub fn completions(matches: &ArgMatches) -> Option<Shell> {
    matches
        .try_get_one::<Shell>(COMPLETIONS)
        .ok()
        .flatten()
        .copied()
}

/// The argument of a field's flag.
fn flag_arg(flag: &Flag) -> Arg {
    let value_parser = match &flag.kind {
        Kind::Bool => one_of(["true", "false"].map(PossibleValue::new), &flag.kind),
        Kind::Choice(choices) => one_of(
            choices.iter().filter_map(|(value, help)| {
                let value = PossibleValue::new(scalar_text(value)?);
                Some(match help {
                    Some(help) => value.help(help.clone()),
                    None => value,
                })
            }),
            &flag.kind,
        ),
        kind => {
            let kind = kind.clone();
            ValueParser::new(move |argument: &str| kind.parse(argument))
        }
    };
    let value_name = match flag.kind {
        Kind::Bool => "BOOL",
        Kind::Integer => "INT",
        Kind::Float => "FLOAT",
        Kind::String => "STRING",
        Kind::Choice(_) => "VALUE",
        Kind::Json => "JSON",
    };
    let mut arg = Arg::new(flag.id())
        .long(flag.name())
        .value_name(value_name)
        .value_parser(value_parser);
    if let Some(help) = &flag.help {
        arg = arg.help(help.clone());
    }
    let default = flag
        .default
        .as_ref()
        .and_then(|value| flag.kind.render(value));
    if let Some(default) = default {
        arg = arg.default_value(default);
    }
    if flag.required {
        arg = arg.required_unless_present_any([INPUT, CONFIG]);
    }
    match flag.kind {
        Kind::Bool => arg.num_args(0..=1).default_missing_value("true"),
        Kind::Integer | Kind::Float | Kind::Json => arg.allow_negative_numbers(true),
        Kind::String | Kind::Choice(_) => arg,
    }
}

/// A value parser that takes one of `values`, parsed as `kind`.
fn one_of(values: impl IntoIterator<Item = PossibleValue>, kind: &Kind) -> ValueParser {
    let kind = kind.clone();
    ValueParser::new(PossibleValuesParser::new(values).try_map(move |text| kind.parse(&text)))
}

/// Sets the field at `path` of `target` to `value`, making `target` and the fields on the way
/// objects.
fn set(target: &mut Value, path: &[&str], value: Value) {
    let [key, rest @ ..] = path else {
        *target = value;
        return;
    };
    if !target.is_object() {
        *target = Value::Object(Map::new());
    }
    if let Value::Object(fields) = target {
        set(fields.entry(*key).or_insert(Value::Null), rest, value);
    }
}

/// Overlays `layer` onto `input`: an object's fields replace those of an object below or are
/// overlaid onto them, `null` included, and any other value replaces `input`.
fn overlay(input: &mut Value, layer: Value) {
    match (input, layer) {
        (Value::Object(input), Value::Object(layer)) => {
            for (key, value) in layer {
                match input.get_mut(&key) {
                    Some(below) => overlay(below, value),
                    None => {
                        input.insert(key, value);
                    }
                }
            }
        }
        (input, layer) => *input = layer,
    }
}

#[cfg(test)]
mod tests {
    use clap::error::ErrorKind;
    use serde_json::json;

    use super::*;
    use crate::DefinitionsFile;

    fn definition(name: &str, schema: Option<Value>, example: Option<Value>) -> JobDefinition {
        JobDefinition {
            name: name.to_string(),
            description: Some(format!("Run {name}")),
            input_schema: schema,
            input_example: example,
        }
    }

    /// A Burn-style configuration's example input, with no schema.
    fn mnist() -> JobDefinition {
        definition(
            "mnist",
            None,
            Some(json!({
                "num_epochs": 10,
                "batch_size": 64,
                "shuffle": true,
                "tag": "baseline",
                "layers": [64, 32],
                "resume_from": null,
                "optimizer": {"lr": 0.001, "weight_decay": 5e-5f32}
            })),
        )
    }

    /// A schema in the form schemars writes, for an input with no example.
    fn prompt() -> JobDefinition {
        definition(
            "prompt",
            Some(json!({
                "type": "object",
                "properties": {
                    "text": {"description": "The prompt.", "type": "string"},
                    "mode": {"type": "string", "enum": ["fast", "exact"]},
                    "limit": {"type": ["integer", "null"]}
                },
                "required": ["text"]
            })),
            None,
        )
    }

    /// The input the job's command line `<job> <args>` gives, its `--config` file read from
    /// `files`, by name.
    fn input_from(
        definition: &JobDefinition,
        args: &[&str],
        files: &[(&str, &str)],
    ) -> Result<Result<Value, ConfigError>, clap::Error> {
        let matches = job_command(definition)
            .try_get_matches_from([definition.name.as_str()].iter().chain(args))?;
        Ok(job_input(&matches, |path| {
            files
                .iter()
                .find(|(name, _)| Path::new(name) == path)
                .map(|(_, contents)| contents.to_string())
                .ok_or_else(|| io::ErrorKind::NotFound.into())
        }))
    }

    /// The input the job's command line `<job> <args>` gives, with no `--config` file to read.
    fn input(definition: &JobDefinition, args: &[&str]) -> Result<Value, clap::Error> {
        input_from(definition, args, &[]).map(|input| input.unwrap())
    }

    /// The job's `--help`.
    fn help(definition: &JobDefinition) -> String {
        job_command(definition).render_long_help().to_string()
    }

    #[test]
    fn the_command_has_a_subcommand_per_job() {
        let jobs = [mnist(), prompt()];
        let mut command = command("program", &jobs);
        command.build();

        let names: Vec<&str> = command.get_subcommands().map(Command::get_name).collect();
        assert_eq!(names, ["mnist", "prompt"]);
        let help = command.render_help().to_string();
        assert!(help.contains("Jobs:"), "{help}");
        assert!(help.contains("Run mnist"), "{help}");
        assert!(help.contains("--completions <SHELL>"), "{help}");
    }

    #[test]
    fn flags_set_fields_under_their_original_keys() {
        let input = input(
            &mnist(),
            &[
                "--num-epochs",
                "2",
                "--optimizer.weight-decay",
                "3e-4",
                "--tag",
                "42",
            ],
        )
        .unwrap();

        assert_eq!(
            input,
            json!({"num_epochs": 2, "optimizer": {"weight_decay": 3e-4}, "tag": "42"})
        );
    }

    #[test]
    fn no_arguments_give_no_input_and_defaults_are_only_shown() {
        assert_eq!(input(&mnist(), &[]).unwrap(), Value::Null);

        let help = help(&mnist());
        for line in [
            "--num-epochs <INT>",
            "[default: 10]",
            "--optimizer.weight-decay <FLOAT>",
            "[default: 0.00005]",
            "--shuffle [<BOOL>]",
            "--layers <JSON>",
            "[default: [64,32]]",
            "--resume-from <JSON>",
            "[default: null]",
            "-c, --config <FILE>",
        ] {
            assert!(help.contains(line), "no {line} in\n{help}");
        }
    }

    #[test]
    fn a_flag_parses_as_its_type() {
        let mnist = mnist();

        assert_eq!(
            input(&mnist, &["--shuffle", "--batch-size", "-1"]).unwrap(),
            json!({"shuffle": true, "batch_size": -1})
        );
        assert_eq!(
            input(&mnist, &["--shuffle", "false"]).unwrap(),
            json!({"shuffle": false})
        );
        for args in [
            &["--num-epochs", "ten"][..],
            &["--num-epochs", "1.5"],
            &["--optimizer.lr", "fast"],
            &["--shuffle", "yes"],
            &["--layers", "[1,"],
        ] {
            let error = input(&mnist, args).unwrap_err();
            assert!(
                matches!(
                    error.kind(),
                    ErrorKind::ValueValidation | ErrorKind::InvalidValue
                ),
                "{args:?}: {error}"
            );
            assert_eq!(error.exit_code(), 2);
        }
        assert_eq!(
            input(&mnist, &["--num-epochs"]).unwrap_err().kind(),
            ErrorKind::InvalidValue
        );
        assert_eq!(
            input(&mnist, &["--epochs", "2"]).unwrap_err().kind(),
            ErrorKind::UnknownArgument
        );
    }

    #[test]
    fn json_literal_flags_take_any_json() {
        assert_eq!(
            input(
                &mnist(),
                &["--layers", "[8]", "--resume-from", r#"{"epoch": 3}"#]
            )
            .unwrap(),
            json!({"layers": [8], "resume_from": {"epoch": 3}})
        );
        assert_eq!(
            input(&mnist(), &["--resume-from", "null"]).unwrap(),
            json!({"resume_from": null})
        );
    }

    #[test]
    fn a_schema_gives_flags_their_values_help_and_requirement() {
        let prompt = prompt();

        assert_eq!(
            input(
                &prompt,
                &["--text", "hi", "--mode", "exact", "--limit", "3"]
            )
            .unwrap(),
            json!({"text": "hi", "mode": "exact", "limit": 3})
        );
        assert_eq!(
            input(&prompt, &["--text", "hi", "--mode", "slow"])
                .unwrap_err()
                .kind(),
            ErrorKind::InvalidValue
        );
        let help = help(&prompt);
        assert!(help.contains("The prompt."), "{help}");
        assert!(help.contains("[possible values: fast, exact]"), "{help}");
    }

    #[test]
    fn a_required_flag_can_be_left_to_the_json_input_or_the_config_file() {
        let prompt = prompt();
        let files = [("config.json", r#"{"text": "from a file"}"#)];

        let missing = input(&prompt, &["--mode", "fast"]).unwrap_err();

        assert_eq!(missing.kind(), ErrorKind::MissingRequiredArgument);
        assert!(missing.to_string().contains("--text <STRING>"), "{missing}");
        assert_eq!(
            input(&prompt, &[r#"{"text": "hi"}"#]).unwrap(),
            json!({"text": "hi"})
        );
        assert_eq!(
            input_from(&prompt, &["-c", "config.json"], &files)
                .unwrap()
                .unwrap(),
            json!({"text": "from a file"})
        );
    }

    #[test]
    fn the_json_input_alone_is_the_input_as_given() {
        let given = json!({"tag": null, "layers": [], "optimizer": {"lr": 0.1}});

        assert_eq!(input(&mnist(), &[&given.to_string()]).unwrap(), given);
        assert_eq!(input(&mnist(), &["7"]).unwrap(), json!(7));
        assert_eq!(input(&mnist(), &["-7"]).unwrap(), json!(-7));
    }

    #[test]
    fn the_config_file_then_the_json_input_then_the_flags_win() {
        let files = [(
            "config.json",
            r#"{"num_epochs": 1, "batch_size": 8, "tag": "file", "optimizer": {"lr": 0.1, "weight_decay": 0.5}}"#,
        )];

        let input = input_from(
            &mnist(),
            &[
                "--num-epochs",
                "3",
                "--config",
                "config.json",
                r#"{"num_epochs": 2, "batch_size": 16, "tag": null, "optimizer": {"lr": 0.2}}"#,
                "--optimizer.lr",
                "0.3",
            ],
            &files,
        )
        .unwrap()
        .unwrap();

        assert_eq!(
            input,
            json!({
                "num_epochs": 3,
                "batch_size": 16,
                "tag": null,
                "optimizer": {"lr": 0.3, "weight_decay": 0.5}
            })
        );
    }

    #[test]
    fn a_repeated_flag_takes_its_last_value() {
        assert_eq!(
            input(&mnist(), &["--num-epochs", "1", "--num-epochs", "2"]).unwrap(),
            json!({"num_epochs": 2})
        );
    }

    #[test]
    fn an_unreadable_or_invalid_config_file_gives_no_input() {
        let files = [("config.toml", "epochs = 3")];
        let error = |path| {
            input_from(&mnist(), &["-c", path], &files)
                .unwrap()
                .unwrap_err()
        };

        let missing = error("missing.json");
        let not_json = error("config.toml");

        assert!(
            matches!(&missing, ConfigError::Unreadable { path, .. } if path == Path::new("missing.json")),
            "{missing:?}"
        );
        assert!(
            missing
                .to_string()
                .starts_with("cannot read the --config file missing.json: ")
        );
        assert!(
            matches!(&not_json, ConfigError::NotJson { .. }),
            "{not_json:?}"
        );
        assert!(
            not_json
                .to_string()
                .starts_with("the --config file config.toml is not JSON: ")
        );
    }

    #[test]
    fn reserved_names_belong_to_the_runner() {
        let job = definition(
            "train",
            None,
            Some(json!({"config": "small", "help": false, "version": 1, "epochs": 3})),
        );
        let files = [("large.json", r#"{"config": "large"}"#)];

        let command = job_command(&job);
        let longs: Vec<&str> = command.get_arguments().filter_map(Arg::get_long).collect();
        assert_eq!(longs, ["epochs", "config"]);
        assert_eq!(
            input_from(&job, &["--config", "large.json"], &files)
                .unwrap()
                .unwrap(),
            json!({"config": "large"})
        );
        assert_eq!(
            input(&job, &["--help"]).unwrap_err().kind(),
            ErrorKind::DisplayHelp
        );
        assert_eq!(
            input(&job, &["--version"]).unwrap_err().kind(),
            ErrorKind::UnknownArgument
        );
    }

    #[test]
    fn completions_names_the_shell_the_command_line_asks_for() {
        let jobs = [mnist()];
        let matches = |args: &[&str]| {
            command("program", &jobs)
                .try_get_matches_from(["program"].iter().chain(args))
                .unwrap()
        };

        assert_eq!(
            completions(&matches(&["--completions", "zsh"])),
            Some(Shell::Zsh)
        );
        assert_eq!(completions(&matches(&["mnist"])), None);
        assert_eq!(
            command("program", &jobs)
                .try_get_matches_from(["program", "--completions", "tcsh"])
                .unwrap_err()
                .kind(),
            ErrorKind::InvalidValue
        );
    }

    #[test]
    fn completions_cover_the_jobs_and_their_flags() {
        let jobs = [mnist(), prompt()];
        let mut script = Vec::new();

        clap_complete::generate(
            Shell::Bash,
            &mut command("program", &jobs),
            "program",
            &mut script,
        );

        let script = String::from_utf8(script).unwrap();
        for word in [
            "mnist",
            "prompt",
            "--optimizer.weight-decay",
            "--config",
            "--completions",
        ] {
            assert!(script.contains(word), "no {word} in the script");
        }
    }

    #[test]
    fn a_definitions_file_builds_the_same_command() {
        let jobs = vec![mnist(), prompt()];
        let file: DefinitionsFile = serde_json::from_value(json!({
            "protocol": 1,
            "sdk_version": "0.10.0",
            "jobs": serde_json::to_value(&jobs).unwrap()
        }))
        .unwrap();

        let mut built = command("program", &jobs);
        let mut read = command("program", &file.jobs);

        for job in ["mnist", "prompt"] {
            let built = built.find_subcommand_mut(job).unwrap().render_help();
            let read = read.find_subcommand_mut(job).unwrap().render_help();
            assert_eq!(built.to_string(), read.to_string());
        }
    }
}
