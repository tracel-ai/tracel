use rustyline::error::ReadlineError;
use rustyline::Editor;
use std::io;

pub enum InputResult {
    Input(String),
    Interrupted,
    Eof,
}

pub trait InputHandler {
    fn read(&mut self, prompt: &str) -> io::Result<InputResult>;
}

impl<H, I> InputHandler for Editor<H, I>
where
    H: rustyline::Helper,
    I: rustyline::history::History,
{
    fn read(&mut self, prompt: &str) -> io::Result<InputResult> {
        match self.readline(&prompt) {
            Ok(s) => {
                self.add_history_entry(&s)
                    .map_err(convert_rustyline_to_io)?;
                Ok(InputResult::Input(s))
            }
            Err(ReadlineError::Eof) => Ok(InputResult::Eof),
            Err(ReadlineError::Interrupted) => Ok(InputResult::Interrupted),
            Err(e) => Err(convert_rustyline_to_io(e)),
        }
    }
}

fn convert_rustyline_to_io(e: ReadlineError) -> io::Error {
    match e {
        ReadlineError::Io(e) => e,
        ReadlineError::Eof => io::Error::new(io::ErrorKind::UnexpectedEof, e),
        ReadlineError::Interrupted => io::Error::new(io::ErrorKind::Interrupted, e),
        #[cfg(unix)]
        ReadlineError::Errno(e) => e.into(),
        ReadlineError::WindowResized => io::Error::new(io::ErrorKind::Other, e),
        e => io::Error::new(io::ErrorKind::Interrupted, e),
    }
}
