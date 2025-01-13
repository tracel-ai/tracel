use std::borrow::Cow::{self, Borrowed, Owned};

use rustyline::completion::FilenameCompleter;
use rustyline::highlight::{CmdKind, Highlighter, MatchingBracketHighlighter};
use rustyline::hint::HistoryHinter;
use rustyline::validate::MatchingBracketValidator;
use rustyline::{CompletionType, Config, EditMode, Editor, ExternalPrinter};
use rustyline::{Completer, Helper, Hinter, Validator};
use rustyline::history::DefaultHistory;
use crate::input_handler::{InputHandler, InputResult};

#[derive(Helper, Completer, Hinter, Validator)]
struct CustomHelper {
    #[rustyline(Completer)]
    completer: FilenameCompleter,
    highlighter: MatchingBracketHighlighter,
    #[rustyline(Validator)]
    validator: MatchingBracketValidator,
    #[rustyline(Hinter)]
    hinter: HistoryHinter,
    colored_prompt: String,
}

impl CustomHelper {
    pub fn new() -> Self {
        Self {
            completer: FilenameCompleter::new(),
            highlighter: MatchingBracketHighlighter::new(),
            validator: MatchingBracketValidator::new(),
            hinter: HistoryHinter {},
            colored_prompt: String::new(),
        }
    }

    fn set_colored_prompt(&mut self, prompt: &str) {
        self.colored_prompt = prompt.to_owned();
    }
}

impl Highlighter for CustomHelper {
    fn highlight<'l>(&self, line: &'l str, pos: usize) -> Cow<'l, str> {
        self.highlighter.highlight(line, pos)
    }

    fn highlight_prompt<'b, 's: 'b, 'p: 'b>(
        &'s self,
        prompt: &'p str,
        default: bool,
    ) -> Cow<'b, str> {
        if default {
            Borrowed(&self.colored_prompt)
        } else {
            Borrowed(prompt)
        }
    }

    fn highlight_hint<'h>(&self, hint: &'h str) -> Cow<'h, str> {
        Owned("\x1b[1m".to_owned() + hint + "\x1b[m")
    }

    fn highlight_char(&self, line: &str, pos: usize, kind: CmdKind) -> bool {
        self.highlighter.highlight_char(line, pos, kind)
    }
}

pub struct CustomEditorHandler {
    last_prompt: String,
    re: regex::Regex,
    editor: Editor<CustomHelper, DefaultHistory>,
}

impl CustomEditorHandler {
    pub fn new() -> anyhow::Result<Self> {
        let re = regex::Regex::new(r"\x1b\[[0-9;]*[a-zA-Z]")?;

        let config = Config::builder()
            .edit_mode(EditMode::Emacs)
            .completion_type(CompletionType::List)
            .build();
        let helper = CustomHelper::new();
        let mut editor = Editor::with_config(config)?;
        editor.set_helper(Some(helper));
        Ok(Self {
            last_prompt: String::new(),
            re,
            editor,
        })
    }

    pub fn create_external_printer(&mut self) -> anyhow::Result<impl ExternalPrinter> {
        self.editor.create_external_printer().map_err(Into::into)
    }

    fn prompt_filter(&mut self, prompt: &str) -> String {
        if prompt == self.last_prompt {
            return prompt.to_owned();
        }
        self.editor.helper_mut().unwrap().set_colored_prompt(prompt);
        let unescaped = self.strip_ansi_escape_codes(prompt);
        unescaped
    }

    fn strip_ansi_escape_codes(&self, s: &str) -> String {
        self.re.replace_all(s, "").to_string()
    }
}

impl InputHandler for CustomEditorHandler {
    fn read(&mut self, prompt: &str) -> std::io::Result<InputResult> {
        let prompt = self.prompt_filter(prompt);
        self.editor.read(&prompt)
    }
}