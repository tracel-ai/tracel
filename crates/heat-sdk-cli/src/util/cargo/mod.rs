//! All of the code in this directory is heavily inspired by the [Cargo source code](https://github.com/rust-lang/cargo)
//! and modified to fit the needs of the heat-sdk-cli project.
//!
//! In files that are entirely copied from Cargo, a link to the original source code is included in the top of the file as a comment.
//! In files that are partially copied from Cargo or include functions/definitons from multiple different files in Cargo, a link to the source code of the original functions/definitions is included in the comments above the copied code.
//!
//! Definitions and functions that are not copied from Cargo do not have a link to the original source code.

mod dependency;
mod features;
mod interning;
mod paths;
mod restricted_names;
mod toml;
mod version;
mod workspace;

pub mod package;
