use cliclack::{clear_screen, confirm};

#[derive(Clone)]
pub struct Terminal {}

impl Terminal {
    pub fn new() -> Self {
        Self {}
    }

    pub fn print_warning(&self, message: &str) {
        cliclack::log::warning(message).expect("To be able to print remark");
    }

    pub fn print(&self, message: &str) {
        cliclack::log::info(message).expect("To be able to print message");
    }

    pub fn print_err(&self, message: &str) {
        cliclack::log::error(message).expect("To be able to print message");
    }

    #[allow(dead_code)]
    pub fn clear(&self) {
        clear_screen().expect("Failed to clear screen");
    }

    pub fn format_url(&self, url: &url::Url) -> String {
        format!("\x1b[1;34m{url}\x1b[0m")
    }

    pub fn confirm(&self, message: &str) -> anyhow::Result<bool> {
        confirm(message).interact().map_err(anyhow::Error::from)
    }

    pub fn command_title(&self, title: &str) {
        let title = format!(" {} {} ", "▶", title);
        cliclack::intro(console::style(title).black().on_green())
            .expect("To be able to print title");
    }

    pub fn finalize(&self, msg: &str) {
        cliclack::outro(console::style(format!(" {} ", msg)).black().on_green())
            .expect("To be able to print message");
    }

    pub fn cancel_finalize(&self, msg: &str) {
        cliclack::outro_cancel(console::style(format!(" {} ", msg)).black().on_red())
            .expect("To be able to print message");
    }

    pub fn input_password(&self, prompt: &str) -> anyhow::Result<String> {
        cliclack::password(prompt)
            .mask('•')
            .interact()
            .map_err(anyhow::Error::from)
    }
}
