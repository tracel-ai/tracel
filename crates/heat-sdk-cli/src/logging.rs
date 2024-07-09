use colored::{Colorize, CustomColor};

pub const BURN_ORANGE: CustomColor = CustomColor {
    r: 254,
    g: 75,
    b: 0,
};

pub fn print_err(err_message: &str) {
    eprintln!(
        "[{}] {}: {}",
        "heat-sdk-cli".custom_color(BURN_ORANGE),
        "error".red().bold(),
        err_message
    );
}

#[macro_export]
macro_rules! print_err {
    ($($arg:tt)*) => {
        $crate::logging::print_err(&format!($($arg)*));
    };
}

pub fn print_warn(warn_message: &str) {
    println!(
        "[{}] {}: {}",
        "heat-sdk-cli".custom_color(BURN_ORANGE),
        "warning".yellow().bold(),
        warn_message
    );
}

#[macro_export]
macro_rules! print_warn {
    ($($arg:tt)*) => {
        $crate::logging::print_warn(&format!($($arg)*));
    };
}

pub fn print_info(info_message: &str) {
    println!(
        "[{}] {}: {}",
        "heat-sdk-cli".custom_color(BURN_ORANGE),
        "info".cyan().bold(),
        info_message
    );
}

#[macro_export]
macro_rules! print_info {
    ($($arg:tt)*) => {
        $crate::logging::print_info(&format!($($arg)*));
    };
}
