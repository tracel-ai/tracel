use console::Term;

pub struct Terminal {
    inner: Term,
}

impl Terminal {
    pub fn new() -> Self {
        Terminal {
            inner: Term::stdout(),
        }
    }

    #[allow(dead_code)]
    pub fn print(&self, message: &str) {
        let _ = self.inner.write_line(message);
    }

    #[allow(dead_code)]
    pub fn clear(&self) {
        self.inner.clear_screen().expect("Failed to clear terminal");
    }

    #[allow(dead_code)]
    pub fn read_line(&self, prompt: &str) -> anyhow::Result<String> {
        let line = self
            .inner
            .read_line_initial_text(prompt)?;
        Ok(line)
    }

    pub fn read_confirmation(&self, prompt: &str) -> anyhow::Result<bool> {
        let response = self.read_line(prompt)?;
        match response.trim().to_lowercase().as_str() {
            "y" | "yes" => Ok(true),
            "n" | "no" => Ok(false),
            _ => Err(anyhow::anyhow!("Invalid response: {}", response)),
        }
    }

    pub fn wait_for_keypress(&self) -> anyhow::Result<()> {
        self.inner
            .read_key()
            .map_err(|e| anyhow::anyhow!("Failed to read keypress: {}", e))?;
        Ok(())
    }

    pub fn url(url: &url::Url) -> String {
        format!("\x1b[1;34m{}\x1b[0m", url)
    }
}
