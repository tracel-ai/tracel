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

    pub fn print(&self, message: &str) {
        println!("{}", message);
    }

    pub fn clear(&self) {
        self.inner.clear_screen().expect("Failed to clear terminal");
    }

    pub fn read_line(&self, prompt: &str) -> String {
        let line = self
            .inner
            .read_line_initial_text(prompt)
            .expect("Failed to read line");
        self.inner.flush().expect("Failed to flush terminal");
        line
    }

    pub fn read_password(&self, prompt: Option<&str>) -> anyhow::Result<String> {
        let pass = dialoguer::Password::new();
        if let Some(prompt) = prompt {
            pass.with_prompt(prompt)
        } else {
            pass
        }
        .interact()
        .map_err(|e| anyhow::anyhow!("Failed to read password: {}", e))
    }
}
