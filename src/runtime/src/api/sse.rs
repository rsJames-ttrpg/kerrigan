/// Parses Server-Sent Events from a byte stream.
/// SSE format: lines of `field: value`, events separated by blank lines.
#[derive(Debug, Default)]
pub struct SseParser {
    buffer: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SseEvent {
    pub event_type: Option<String>,
    pub data: String,
}

impl SseParser {
    pub fn new() -> Self {
        Self {
            buffer: String::new(),
        }
    }

    /// Feed bytes into the parser, returning any complete events.
    pub fn feed(&mut self, chunk: &str) -> Vec<SseEvent> {
        self.buffer.push_str(chunk);
        let mut events = Vec::new();
        while let Some(event) = self.try_parse_event() {
            events.push(event);
        }
        events
    }

    fn try_parse_event(&mut self) -> Option<SseEvent> {
        // Look for double newline (event boundary)
        let boundary = self.buffer.find("\n\n")?;
        let raw = self.buffer[..boundary].to_string();
        self.buffer = self.buffer[boundary + 2..].to_string();

        let mut event_type = None;
        let mut data_lines = Vec::new();

        for line in raw.lines() {
            if let Some(value) = line.strip_prefix("event: ") {
                event_type = Some(value.to_string());
            } else if let Some(value) = line.strip_prefix("data: ") {
                data_lines.push(value);
            } else if let Some(value) = line.strip_prefix("data:") {
                // "data:" with no space — empty or no-space data line
                data_lines.push(value);
            }
            // Ignore other fields (id:, retry:, comments)
        }

        if data_lines.is_empty() && event_type.is_none() {
            return None;
        }

        Some(SseEvent {
            event_type,
            data: data_lines.join("\n"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_single_event() {
        let mut parser = SseParser::new();
        let events = parser.feed("event: message\ndata: hello\n\n");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type.as_deref(), Some("message"));
        assert_eq!(events[0].data, "hello");
    }

    #[test]
    fn test_multi_line_data() {
        let mut parser = SseParser::new();
        let events = parser.feed("data: line1\ndata: line2\n\n");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data, "line1\nline2");
    }

    #[test]
    fn test_chunked_delivery() {
        let mut parser = SseParser::new();
        assert!(parser.feed("event: msg\n").is_empty());
        assert!(parser.feed("data: part").is_empty());
        let events = parser.feed("ial\n\n");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data, "partial");
    }

    #[test]
    fn test_multiple_events() {
        let mut parser = SseParser::new();
        let events = parser.feed("data: first\n\ndata: second\n\n");
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].data, "first");
        assert_eq!(events[1].data, "second");
    }

    #[test]
    fn test_event_without_type() {
        let mut parser = SseParser::new();
        let events = parser.feed("data: just data\n\n");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, None);
        assert_eq!(events[0].data, "just data");
    }

    #[test]
    fn test_empty_data_line() {
        let mut parser = SseParser::new();
        let events = parser.feed("data:\n\n");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data, "");
    }

    #[test]
    fn test_ignores_comments_and_unknown_fields() {
        let mut parser = SseParser::new();
        let events = parser.feed(": this is a comment\nid: 123\nretry: 5000\ndata: hello\n\n");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data, "hello");
    }
}
