use console::Term;
use std::io::Write;

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
        let line = self.inner.read_line_initial_text(prompt)?;
        Ok(line)
    }

    pub fn read_password(&mut self, prompt: &str) -> anyhow::Result<String> {
        let prompt = format!("{} {prompt}: ", console::style("?").green());
        self.inner
            .write(prompt.as_bytes())
            .map_err(|e| anyhow::anyhow!("Failed to write prompt: {}", e))?;
        let password = self
            .inner
            .read_secure_line()
            .map_err(|e| anyhow::anyhow!("Failed to read password: {}", e))?;

        self.inner.clear_last_lines(1)?;
        self.inner
            .write_line(&format!("{}{}", &prompt, "********"))?;

        Ok(password)
    }

    pub fn url(url: &url::Url) -> String {
        format!("\x1b[1;34m{}\x1b[0m", url)
    }
}
