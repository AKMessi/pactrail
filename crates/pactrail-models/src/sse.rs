use thiserror::Error;

const MAX_SSE_LINE_BYTES: usize = 1024 * 1024;
const MAX_SSE_EVENT_BYTES: usize = 1024 * 1024;
const MAX_SSE_EVENT_NAME_BYTES: usize = 256;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SseEvent {
    pub event: Option<String>,
    pub data: String,
}

/// Incremental UTF-8 server-sent-event decoder with strict memory bounds.
pub(crate) struct SseDecoder {
    pending_line: Vec<u8>,
    event_name: Option<String>,
    data: String,
    has_data: bool,
}

impl SseDecoder {
    pub(crate) const fn new() -> Self {
        Self {
            pending_line: Vec::new(),
            event_name: None,
            data: String::new(),
            has_data: false,
        }
    }

    pub(crate) fn push(&mut self, chunk: &[u8]) -> Result<Vec<SseEvent>, SseError> {
        if self.pending_line.len().saturating_add(chunk.len()) > MAX_SSE_LINE_BYTES
            && !chunk.contains(&b'\n')
        {
            return Err(SseError::LineTooLarge);
        }
        self.pending_line.extend_from_slice(chunk);
        let mut events = Vec::new();
        while let Some(end) = self.pending_line.iter().position(|byte| *byte == b'\n') {
            let mut line = self.pending_line.drain(..=end).collect::<Vec<_>>();
            let _newline = line.pop();
            if line.last() == Some(&b'\r') {
                let _carriage_return = line.pop();
            }
            if line.len() > MAX_SSE_LINE_BYTES {
                return Err(SseError::LineTooLarge);
            }
            if let Some(event) = self.process_line(&line)? {
                events.push(event);
            }
        }
        if self.pending_line.len() > MAX_SSE_LINE_BYTES {
            return Err(SseError::LineTooLarge);
        }
        Ok(events)
    }

    pub(crate) fn finish(self) -> Result<(), SseError> {
        if self.pending_line.is_empty()
            && self.event_name.is_none()
            && self.data.is_empty()
            && !self.has_data
        {
            Ok(())
        } else {
            Err(SseError::IncompleteEvent)
        }
    }

    fn process_line(&mut self, line: &[u8]) -> Result<Option<SseEvent>, SseError> {
        if line.is_empty() {
            return Ok(self.dispatch());
        }
        if line.first() == Some(&b':') {
            return Ok(None);
        }
        let line = std::str::from_utf8(line).map_err(|_| SseError::InvalidUtf8)?;
        let (field, value) = line.split_once(':').map_or((line, ""), |(field, value)| {
            (field, value.strip_prefix(' ').unwrap_or(value))
        });
        match field {
            "event" => {
                if value.len() > MAX_SSE_EVENT_NAME_BYTES || value.contains('\0') {
                    return Err(SseError::InvalidEventName);
                }
                self.event_name = Some(value.to_owned());
            }
            "data" => {
                let separator = usize::from(self.has_data);
                if self
                    .data
                    .len()
                    .saturating_add(separator)
                    .saturating_add(value.len())
                    > MAX_SSE_EVENT_BYTES
                {
                    return Err(SseError::EventTooLarge);
                }
                if self.has_data {
                    self.data.push('\n');
                }
                self.data.push_str(value);
                self.has_data = true;
            }
            // `id`, `retry`, and future extension fields have no bearing on
            // Pactrail's stateless request accumulator.
            _ => {}
        }
        Ok(None)
    }

    fn dispatch(&mut self) -> Option<SseEvent> {
        if !self.has_data {
            self.event_name = None;
            return None;
        }
        Some(SseEvent {
            event: self.event_name.take(),
            data: std::mem::take(&mut self.data),
        })
        .inspect(|_| self.has_data = false)
    }
}

#[derive(Debug, Error)]
pub(crate) enum SseError {
    #[error("SSE line exceeded its byte limit")]
    LineTooLarge,
    #[error("SSE event exceeded its byte limit")]
    EventTooLarge,
    #[error("SSE event name is invalid")]
    InvalidEventName,
    #[error("SSE stream contained invalid UTF-8")]
    InvalidUtf8,
    #[error("SSE stream ended inside an event")]
    IncompleteEvent,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_fragmented_crlf_multiline_events_and_ignores_comments() {
        let input = b": ping\r\nevent: update\r\ndata: first\r\ndata: second\r\n\r\n";
        let mut decoder = SseDecoder::new();
        let mut events = Vec::new();
        for byte in input {
            events.extend(
                decoder
                    .push(std::slice::from_ref(byte))
                    .unwrap_or_else(|error| unreachable!("valid stream: {error}")),
            );
        }
        decoder
            .finish()
            .unwrap_or_else(|error| unreachable!("complete stream: {error}"));
        assert_eq!(
            events,
            vec![SseEvent {
                event: Some("update".to_owned()),
                data: "first\nsecond".to_owned(),
            }]
        );
    }

    #[test]
    fn rejects_invalid_utf8_oversized_lines_and_incomplete_events() {
        let mut invalid = SseDecoder::new();
        assert!(matches!(
            invalid.push(&[b'd', b'a', b't', b'a', b':', 0xff, b'\n']),
            Err(SseError::InvalidUtf8)
        ));

        let mut oversized = SseDecoder::new();
        assert!(matches!(
            oversized.push(&vec![b'x'; MAX_SSE_LINE_BYTES + 1]),
            Err(SseError::LineTooLarge)
        ));

        let mut incomplete = SseDecoder::new();
        incomplete
            .push(b"data: unfinished\n")
            .unwrap_or_else(|error| unreachable!("line is valid: {error}"));
        assert!(matches!(
            incomplete.finish(),
            Err(SseError::IncompleteEvent)
        ));
    }

    #[test]
    fn ignores_empty_events_and_unknown_fields() {
        let mut decoder = SseDecoder::new();
        let events = decoder
            .push(b"event: ping\nretry: 10\nid: ignored\n\ndata: ok\n\n")
            .unwrap_or_else(|error| unreachable!("valid stream: {error}"));
        assert_eq!(
            events,
            vec![SseEvent {
                event: None,
                data: "ok".to_owned(),
            }]
        );
        decoder
            .finish()
            .unwrap_or_else(|error| unreachable!("complete stream: {error}"));
    }
}
