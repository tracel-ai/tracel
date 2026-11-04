//! The command line of a program's jobs, generated from their input schemas, and the JSON input
//! it gives.
//!
//! [`command`] builds the command line `<program> <job> [<input-json>] [<flags>]` from each job's
//! [`JobDefinition`](crate::JobDefinition): its name, description, input schema and example
//! input, which is its default input. [`job_input`] reads a job's input back from the arguments
//! it parsed. The command line depends on nothing else, so a program that knows another program's
//! jobs parses the same flags as that program does: a job binary parses its own command line with
//! it, and `tracel run <job> --<flag>` parses the job's flags before it launches the binary.
//!
//! ## Flags
//!
//! A job whose input schema describes an object has a flag per field of the input, typed by the
//! schema: a boolean, an integer, a number, a string, or one of the values the schema lists. A
//! job without an input schema has no flags: its input is the JSON document or the `--config`
//! file. The schema is read in the form schemars writes it:
//!
//! - An object's `properties` are its fields, and a nested object's fields are flags too: the
//!   field `weight_decay` of `optimizer` is `--optimizer.weight-decay`, the keys joined with dots
//!   and `_` written as `-`.
//! - `$ref` is followed into the schema's `$defs` or `definitions`.
//! - `enum`, `const`, and `oneOf` or `anyOf` of constants, such as the unit variants of an enum,
//!   are the values a flag takes, each with its description.
//! - An optional field, `type: [T, "null"]` or `anyOf` of `T` and `null`, takes the values of `T`.
//! - Any other field, such as an array, a map, an optional object or an enum with data, takes a
//!   JSON literal, such as `--layers '[64, 32]'`.
//!
//! A field's `description` is its flag's help, and its value in the job's default input, or else
//! its schema `default`, is the flag's shown default. A field the schema requires is a required
//! flag unless it has a default or the JSON document or `--config` is given. A boolean flag given
//! no value is `true`. The schema's other constraints, such as `minimum` or `pattern`, are not
//! checked here: the job's input is checked as a whole once it is resolved.
//!
//! `--help`, `--version` and `--config` belong to the command line, so a field named `help`,
//! `version` or `config` has no flag; neither does a field whose key is not made of letters,
//! digits, `_` and `-`, or whose flag name an earlier field took. Such a field is set through the
//! JSON document or `--config`.
//!
//! ## Input
//!
//! A job's input is its default input, then the `--config` file, then the JSON document, then the
//! flags, each merged onto the one before, so a later one wins. [`job_input`] gives the last
//! three overlaid, which the job merges onto its default input. It does no I/O of its own: it
//! reads the `--config` file through the function its caller gives.

mod command;
mod flags;

pub use command::{ConfigError, command, completions, job_command, job_input};
