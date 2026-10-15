//! Minimal Server-Sent Events parsing for the runner stream.

/// One complete SSE frame: an event name and its (possibly multi-line) data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseFrame {
    pub event: String,
    pub data: String,
}

/// Incremental line-by-line SSE parser.
///
/// Feed lines (without their terminator); a blank line completes the pending frame. Comment
/// lines (`:`, used by the station as keep-alives) and fields other than `event`/`data` are
/// ignored. Per the SSE spec, an absent event name defaults to `message` and multiple `data`
/// lines are joined with `\n`.
#[derive(Debug, Default)]
pub struct SseParser {
    event: Option<String>,
    data: Vec<String>,
}

impl SseParser {
    pub fn push_line(&mut self, line: &str) -> Option<SseFrame> {
        if line.is_empty() {
            if self.data.is_empty() {
                self.event = None;
                return None;
            }
            let frame = SseFrame {
                event: self.event.take().unwrap_or_else(|| "message".to_string()),
                data: std::mem::take(&mut self.data).join("\n"),
            };
            return Some(frame);
        }
        if line.starts_with(':') {
            return None;
        }

        let (field, value) = match line.split_once(':') {
            Some((field, value)) => (field, value.strip_prefix(' ').unwrap_or(value)),
            None => (line, ""),
        };
        match field {
            "event" => self.event = Some(value.to_string()),
            "data" => self.data.push(value.to_string()),
            _ => {}
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(lines: &[&str]) -> Vec<SseFrame> {
        let mut parser = SseParser::default();
        lines
            .iter()
            .filter_map(|line| parser.push_line(line))
            .collect()
    }

    #[test]
    fn a_blank_line_completes_the_frame_of_its_event_and_data() {
        let frames = parse(&["event: job", "data: {\"id\":1}", ""]);

        assert_eq!(
            frames,
            vec![SseFrame {
                event: "job".to_string(),
                data: "{\"id\":1}".to_string(),
            }]
        );
    }

    #[test]
    fn a_frame_without_an_event_name_is_a_message() {
        let frames = parse(&["data: hello", ""]);

        assert_eq!(frames[0].event, "message");
    }

    #[test]
    fn data_lines_join_with_a_newline() {
        let frames = parse(&["event: job", "data: a", "data: b", ""]);

        assert_eq!(frames[0].data, "a\nb");
    }

    #[test]
    fn comments_and_other_fields_are_ignored() {
        let frames = parse(&[": keep-alive", "id: 42", "retry: 1000", ""]);

        assert!(frames.is_empty());
    }

    #[test]
    fn a_value_needs_no_leading_space() {
        let frames = parse(&["event:job", "data:x", ""]);

        assert_eq!(
            frames,
            [SseFrame {
                event: "job".to_string(),
                data: "x".to_string(),
            }]
        );
    }

    #[test]
    fn each_frame_starts_afresh() {
        let frames = parse(&["event: a", "data: 1", "", "data: 2", ""]);

        assert_eq!(
            frames[1],
            SseFrame {
                event: "message".to_string(),
                data: "2".to_string(),
            }
        );
    }
}
