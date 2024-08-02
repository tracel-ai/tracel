//! All of the code in this directory is heavily inspired by the [Cargo source code](https://github.com/rust-lang/cargo)
//! and modified to fit the needs of the heat-sdk-cli project.

mod dependency;
mod features;
mod interning;
mod paths;
mod restricted_names;
mod toml;
mod version;
mod workspace;

pub mod package;
