//! The command line of a program's jobs, built from each job's name, description, input schema
//! and default input.

use std::io;
use std::path::{Path, PathBuf};

use clap::builder::{
    PathBufValueParser, PossibleValue, PossibleValuesParser, TypedValueParser, ValueParser,
};
use clap::parser::ValueSource;
use clap::{Arg, ArgMatches, Command, ValueHint, value_parser};
use clap_complete::Shell;
use serde_json::{Map, Value};

use super::flags::{Flag, Kind, flags};
use crate::JobDefinition;

/// The id of a job's input argument, which no field's flag has: field ids have no `:`.
const INPUT: &str = "tracel:input";
/// The id of a job's `--config` argument.
const CONFIG: &str = "tracel:config";
/// The id of the program's `--completions` argument.
const COMPLETIONS: &str = "tracel:completions";

/// The command line of the program `name` that runs `jobs`:
/// `<name> <job> [<input-json>] [--<field> <value>...]`, with one subcommand per job, as
/// [`job_command`] builds it.
///
/// `<name> --completions <SHELL>` asks for the program's completion script, which
/// [`completions`] reads and [`clap_complete::generate`] writes from this command. A program that
/// gives its version adds `--version`.
pub fn command<'a>(
    name: impl Into<String>,
    jobs: impl IntoIterator<Item = &'a JobDefinition>,
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
        .subcommands(jobs.into_iter().map(job_command))
}

/// The command line of `job`, named after it, which [`job_input`] reads the job's input from.
///
/// It takes three arguments, each merged onto the one before, so a later one wins:
///
/// 1. `-c, --config <FILE>`: a file holding a JSON document.
/// 2. `[INPUT]`: a JSON document, which launchers pass.
/// 3. One flag per field of the input, which sets that field, when the job's input schema
///    describes an object, as the [`cli`](crate::cli#flags) module describes.
pub fn job_command(job: &JobDefinition) -> Command {
    let command = Command::new(job.name.clone())
        .args_override_self(true)
        .args(flags(job).iter().map(flag_arg))
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
    match &job.description {
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
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// The file cannot be read.
    #[error("cannot read the --config file {}: {source}", path.display())]
    Unreadable {
        /// The path the command line gives.
        path: PathBuf,
        /// Why it cannot be read.
        source: io::Error,
    },
    /// The file is not a JSON document.
    #[error("the --config file {} is not JSON: {source}", path.display())]
    NotJson {
        /// The path the command line gives.
        path: PathBuf,
        /// Why it is not JSON.
        source: serde_json::Error,
    },
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
    let kind = flag.kind.clone();
    let value_parser = match &flag.kind {
        Kind::Bool => one_of(["true", "false"].map(PossibleValue::new), kind),
        Kind::Choice(choices) => one_of(
            choices.iter().map(|choice| {
                let value = PossibleValue::new(choice.text.clone());
                match &choice.help {
                    Some(help) => value.help(help.clone()),
                    None => value,
                }
            }),
            kind,
        ),
        _ => ValueParser::new(move |argument: &str| kind.parse(argument)),
    };
    let value_name = match flag.kind {
        Kind::Bool => "BOOL",
        Kind::Integer => "INT",
        Kind::Number => "FLOAT",
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
    if let Some(default) = flag
        .default
        .as_ref()
        .and_then(|value| flag.kind.render(value))
    {
        arg = arg.default_value(default);
    }
    if flag.required {
        arg = arg.required_unless_present_any([INPUT, CONFIG]);
    }
    match flag.kind {
        Kind::Bool => arg
            .num_args(0..=1)
            .require_equals(true)
            .default_missing_value("true"),
        Kind::Integer | Kind::Number | Kind::Json => arg.allow_negative_numbers(true),
        Kind::String | Kind::Choice(_) => arg,
    }
}

/// A value parser that takes one of `values`, parsed as `kind`.
fn one_of(values: impl IntoIterator<Item = PossibleValue>, kind: Kind) -> ValueParser {
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
    use rstest::rstest;
    use schemars::JsonSchema;
    use serde::Serialize;
    use serde_json::json;

    use super::*;

    #[derive(Serialize, JsonSchema)]
    struct Training {
        num_epochs: u32,
        batch_size: u32,
        shuffle: bool,
        tag: String,
        layers: Vec<u32>,
        mode: Mode,
        resume_from: Option<Resume>,
        optimizer: Optimizer,
    }

    #[derive(Serialize, JsonSchema)]
    struct Optimizer {
        lr: f64,
        weight_decay: f32,
    }

    #[derive(Serialize, JsonSchema)]
    struct Resume {
        epoch: u32,
    }

    fn train() -> JobDefinition {
        let default = Training {
            num_epochs: 10,
            batch_size: 64,
            shuffle: true,
            tag: "baseline".to_string(),
            layers: vec![64, 32],
            mode: Mode::Fast,
            resume_from: None,
            optimizer: Optimizer {
                lr: 0.001,
                weight_decay: 5e-5,
            },
        };
        JobDefinition {
            name: "train".to_string(),
            description: Some("Train the model".to_string()),
            input_schema: Some(schemars::schema_for!(Training).to_value()),
            input_example: Some(serde_json::to_value(default).unwrap()),
        }
    }

    #[derive(JsonSchema)]
    #[expect(dead_code)]
    struct Prompt {
        text: String,
        mode: Mode,
    }

    #[derive(Serialize, JsonSchema)]
    #[serde(rename_all = "snake_case")]
    #[expect(dead_code)]
    enum Mode {
        Fast,
        Exact,
    }

    fn prompt() -> JobDefinition {
        JobDefinition {
            name: "prompt".to_string(),
            description: Some("Answer a prompt".to_string()),
            input_schema: Some(schemars::schema_for!(Prompt).to_value()),
            input_example: None,
        }
    }

    fn evaluate() -> JobDefinition {
        JobDefinition {
            name: "evaluate".to_string(),
            description: None,
            input_schema: None,
            input_example: Some(json!({"split": "test", "limit": 10})),
        }
    }

    fn named_like_flags() -> JobDefinition {
        JobDefinition {
            name: "train".to_string(),
            description: None,
            input_schema: Some(json!({
                "type": "object",
                "properties": {
                    "config": {"type": "string"},
                    "version": {"type": "integer"}
                }
            })),
            input_example: None,
        }
    }

    fn input_from(
        job: &JobDefinition,
        args: &[&str],
        files: &[(&str, &str)],
    ) -> Result<Result<Value, ConfigError>, clap::Error> {
        let matches =
            job_command(job).try_get_matches_from([job.name.as_str()].iter().chain(args))?;
        Ok(job_input(&matches, |path| {
            files
                .iter()
                .find(|(name, _)| Path::new(name) == path)
                .map(|(_, contents)| contents.to_string())
                .ok_or_else(|| io::ErrorKind::NotFound.into())
        }))
    }

    fn input(job: &JobDefinition, args: &[&str]) -> Result<Value, clap::Error> {
        input_from(job, args, &[]).map(|input| input.unwrap())
    }

    #[test]
    fn each_job_is_a_subcommand_about_its_description() {
        let jobs = [train(), prompt(), evaluate()];

        let command = command("program", &jobs);

        let subcommands: Vec<(&str, Option<String>)> = command
            .get_subcommands()
            .map(|job| (job.get_name(), job.get_about().map(ToString::to_string)))
            .collect();
        let expected: Vec<(&str, Option<String>)> = jobs
            .iter()
            .map(|job| (job.name.as_str(), job.description.clone()))
            .collect();
        assert_eq!(subcommands, expected);
    }

    #[rstest]
    #[case::at_the_top(&["--num-epochs", "2"], json!({"num_epochs": 2}))]
    #[case::in_its_object(
        &["--optimizer.weight-decay", "0.5"],
        json!({"optimizer": {"weight_decay": 0.5}})
    )]
    #[case::beside_another_in_its_object(
        &["--optimizer.lr", "0.1", "--optimizer.weight-decay", "0.5"],
        json!({"optimizer": {"lr": 0.1, "weight_decay": 0.5}})
    )]
    fn a_flag_sets_its_field_under_its_original_keys(
        #[case] args: &[&str],
        #[case] expected: Value,
    ) {
        assert_eq!(input(&train(), args).unwrap(), expected);
    }

    #[test]
    fn a_choice_flag_sets_its_field_to_the_value_chosen() {
        assert_eq!(
            input(&train(), &["--mode", "exact"]).unwrap(),
            json!({"mode": "exact"})
        );
    }

    #[test]
    fn the_defaults_alone_give_no_input() {
        assert_eq!(input(&train(), &[]).unwrap(), Value::Null);
    }

    #[rstest]
    #[case::integer(&["--batch-size", "-1"], json!({"batch_size": -1}))]
    #[case::number(&["--optimizer.lr", "-0.5"], json!({"optimizer": {"lr": -0.5}}))]
    #[case::json(&["--layers", "-1"], json!({"layers": -1}))]
    fn a_numeric_or_json_flag_takes_a_negative_number(
        #[case] args: &[&str],
        #[case] expected: Value,
    ) {
        assert_eq!(input(&train(), args).unwrap(), expected);
    }

    #[rstest]
    #[case::integer(&["--num-epochs", "ten"])]
    #[case::bool(&["--shuffle=yes"])]
    #[case::choice(&["--mode", "slow"])]
    fn a_flag_given_a_value_its_field_does_not_take_is_rejected(#[case] args: &[&str]) {
        assert!(input(&train(), args).is_err());
    }

    #[rstest]
    #[case::alone(&["--shuffle"], json!({"shuffle": true}))]
    #[case::after_an_equals_sign(&["--shuffle=false"], json!({"shuffle": false}))]
    #[case::before_the_json_input(
        &["--shuffle", r#"{"num_epochs": 2}"#],
        json!({"num_epochs": 2, "shuffle": true})
    )]
    fn a_bool_flag_takes_a_value_only_after_an_equals_sign(
        #[case] args: &[&str],
        #[case] expected: Value,
    ) {
        assert_eq!(input(&train(), args).unwrap(), expected);
    }

    #[rstest]
    #[case::an_object(json!({"epoch": 3}))]
    #[case::null(Value::Null)]
    fn a_json_flag_sets_its_field_to_the_json_it_is_given(#[case] value: Value) {
        let input = input(&train(), &["--resume-from", &value.to_string()]).unwrap();

        assert_eq!(input, json!({"resume_from": value}));
    }

    #[test]
    fn a_required_flag_left_out_is_rejected() {
        assert!(input(&prompt(), &["--mode", "fast"]).is_err());
    }

    #[rstest]
    #[case::the_json_input(&[r#"{"text": "hi"}"#])]
    #[case::the_config_file(&["--config", "config.json"])]
    fn a_required_flag_can_be_left_to(#[case] args: &[&str]) {
        let files = [("config.json", r#"{"text": "from a file"}"#)];

        assert!(input_from(&prompt(), args, &files).is_ok());
    }

    #[rstest]
    #[case::an_object(json!({"tag": null, "layers": [], "optimizer": {"lr": 0.1}}))]
    #[case::a_number(json!(7))]
    #[case::a_negative_number(json!(-7))]
    fn the_json_input_alone_is_the_input_as_given(#[case] given: Value) {
        assert_eq!(input(&train(), &[&given.to_string()]).unwrap(), given);
    }

    #[test]
    fn an_input_that_is_not_json_is_rejected() {
        assert!(input(&train(), &["{"]).is_err());
    }

    #[test]
    fn the_config_file_then_the_json_input_then_the_flags_win() {
        let files = [(
            "config.json",
            r#"{"num_epochs": 1, "batch_size": 8, "tag": "file"}"#,
        )];
        let args = [
            "--num-epochs",
            "3",
            "--config",
            "config.json",
            r#"{"num_epochs": 2, "batch_size": 16}"#,
        ];

        let input = input_from(&train(), &args, &files).unwrap().unwrap();

        assert_eq!(
            input,
            json!({"num_epochs": 3, "batch_size": 16, "tag": "file"})
        );
    }

    #[rstest]
    #[case::a_value_onto_an_object(json!({"num_epochs": 1}), json!(7))]
    #[case::an_object_onto_a_value(json!(7), json!({"num_epochs": 3}))]
    fn a_layer_replaces_what_is_below_unless_both_are_objects(
        #[case] mut below: Value,
        #[case] layer: Value,
    ) {
        overlay(&mut below, layer.clone());

        assert_eq!(below, layer);
    }

    #[test]
    fn an_object_layer_is_overlaid_field_by_field() {
        let mut below = json!({"num_epochs": 1, "optimizer": {"lr": 0.1, "weight_decay": 0.5}});

        overlay(&mut below, json!({"optimizer": {"lr": 0.2}}));

        assert_eq!(
            below,
            json!({"num_epochs": 1, "optimizer": {"lr": 0.2, "weight_decay": 0.5}})
        );
    }

    #[test]
    fn a_null_field_is_kept_rather_than_applied() {
        let mut below = json!({"num_epochs": 1, "tag": "file"});

        overlay(&mut below, json!({"tag": null}));

        assert_eq!(below, json!({"num_epochs": 1, "tag": null}));
    }

    #[test]
    fn a_repeated_flag_takes_its_last_value() {
        assert_eq!(
            input(&train(), &["--num-epochs", "1", "--num-epochs", "2"]).unwrap(),
            json!({"num_epochs": 2})
        );
    }

    #[test]
    fn an_unreadable_config_file_is_an_error_naming_it() {
        let missing = "missing.json";

        let error = input_from(&train(), &["--config", missing], &[])
            .unwrap()
            .unwrap_err();

        assert!(
            matches!(error, ConfigError::Unreadable { path, .. } if path == Path::new(missing))
        );
    }

    #[test]
    fn a_config_file_that_is_not_json_is_an_error_naming_it() {
        let toml = "config.toml";

        let error = input_from(&train(), &["--config", toml], &[(toml, "epochs = 3")])
            .unwrap()
            .unwrap_err();

        assert!(matches!(error, ConfigError::NotJson { path, .. } if path == Path::new(toml)));
    }

    #[test]
    fn the_config_flag_names_the_config_file_beside_a_field_named_config() {
        let files = [("large.json", r#"{"config": "large"}"#)];

        let input = input_from(&named_like_flags(), &["--config", "large.json"], &files)
            .unwrap()
            .unwrap();

        assert_eq!(input, json!({"config": "large"}));
    }

    #[test]
    fn a_field_named_version_is_set_by_its_flag() {
        assert_eq!(
            input(&named_like_flags(), &["--version", "3"]).unwrap(),
            json!({"version": 3})
        );
    }

    #[rstest]
    #[case::when_asked(&["--completions", "zsh"], Some(Shell::Zsh))]
    #[case::when_a_job_is_named(&["train"], None)]
    fn completions_names_the_shell_the_command_line_asks_for(
        #[case] args: &[&str],
        #[case] shell: Option<Shell>,
    ) {
        let jobs = [train()];

        let matches = command("program", &jobs)
            .try_get_matches_from(["program"].iter().chain(args))
            .unwrap();

        assert_eq!(completions(&matches), shell);
    }
}
