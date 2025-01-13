use cargo_metadata::Message;
use indicatif::ProgressBar;
use std::time::Duration;

pub struct CargoBuildRenderer {
    progress_bar: ProgressBar,
}

impl CargoBuildRenderer {

    pub fn new() -> Self {
        let progress_bar = indicatif::ProgressBar::new_spinner();
        progress_bar.enable_steady_tick(Duration::from_millis(100));
        progress_bar.set_style(
            indicatif::ProgressStyle::with_template("{spinner:.dim.bold} cargo build: {wide_msg}")
                .unwrap()
                .tick_chars("/|\\- "),
        );
        Self { progress_bar }
    }

    pub fn render(&mut self, message: Message) {
        match message {
            Message::CompilerArtifact(msg) => {
                // collect the final binaries
                if !msg.fresh {
                    let pkgid = msg.package_id;
                    self.progress_bar.set_message(format!("Building {}", pkgid));

                    if let Some(executable) = msg.executable {
                        self.progress_bar.set_message(format!("Built {}", executable));
                    }
                }
            }
            Message::CompilerMessage(msg) => {
                let message = msg.message;
                let severity = match message.level {
                    cargo_metadata::diagnostic::DiagnosticLevel::Error => "❌",
                    cargo_metadata::diagnostic::DiagnosticLevel::Warning => "⚠️ ",
                    cargo_metadata::diagnostic::DiagnosticLevel::Note => "ℹ️ ",
                    _ => return,
                };
                self.progress_bar.set_message(format!(
                    "{} {}: {}",
                    severity, message.code.map_or("".to_string(), |c| c.code)
                    , message.message
                ));
                // set message for the compilation task
            }
            Message::BuildScriptExecuted(msg) => {
                self.progress_bar.set_message("Build script executed...");
            }
            Message::TextLine(msg) => {
                self.progress_bar.set_message(format!("{}", msg));
            }
            _ => ()
        }
    }
    
    pub fn tick(&mut self) {
        self.progress_bar.tick();
    }
    
    pub fn finish(&mut self) {
        self.progress_bar.finish_and_clear();
    }
}