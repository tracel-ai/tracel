mod app;

mod child_cli;
pub mod shell;
mod input_handler;
mod rustyline_handler;
mod build_renderer;
mod command_set;
mod command_handler;
mod util;

pub use app::{
    // get_cli_arg_matches,
    // shell_main,
    // cli_main,
    main,
};

pub use command_set::{
    ShellCommandSet,
};

pub use clap;


#[doc(hidden)]
#[macro_use]
pub mod __internals {

    use std::sync::OnceLock;

    static BINARY_NAME: OnceLock<String> = OnceLock::new();

    /// Internal function to set the binary name, only once.
    #[doc(hidden)]
    pub fn set_binary_name(name: &str) {
        // This will only set the value once, subsequent calls will be ignored.
        BINARY_NAME.set(name.to_string()).ok();
    }

    /// Public API to get the binary name, panicking if it hasn't been set.
    pub fn get_binary_name() -> Option<&'static str> {
        BINARY_NAME.get().map(|s| s.as_str())
    }

    #[macro_export]
    macro_rules! capture_bin_name {
        () => {
            env!("CARGO_BIN_NAME")
        };
    }
    #[macro_export]
    macro_rules! capture_crate_name {
        () => {
            env!("CARGO_CRATE_NAME")
        };
    }
}