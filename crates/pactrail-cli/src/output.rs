use std::io::Write;

/// Writes user output without bypassing I/O error handling.
pub fn write_stdout(value: &str) -> std::io::Result<()> {
    let stdout = std::io::stdout();
    let mut lock = stdout.lock();
    lock.write_all(value.as_bytes())?;
    lock.flush()
}

/// Writes terminal-facing output after replacing control characters.
pub fn write_human_stdout(value: &str) -> std::io::Result<()> {
    write_stdout(&sanitize_terminal_text(value))
}

/// Writes diagnostics without using unstructured print macros.
pub fn write_stderr(value: &str) -> std::io::Result<()> {
    let stderr = std::io::stderr();
    let mut lock = stderr.lock();
    let sanitized = sanitize_terminal_text(value);
    lock.write_all(sanitized.as_bytes())?;
    lock.flush()
}

/// Escapes C1 controls that JSON permits as literal Unicode while preserving
/// the value produced when a JSON parser decodes the output.
#[must_use]
pub fn escape_json_terminal_controls(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        if ('\u{7f}'..='\u{9f}').contains(&character) {
            let code = u32::from(character);
            escaped.push_str("\\u00");
            escaped.push(char::from_digit((code >> 4) & 0x0f, 16).unwrap_or('0'));
            escaped.push(char::from_digit(code & 0x0f, 16).unwrap_or('0'));
        } else {
            escaped.push(character);
        }
    }
    escaped
}

pub(crate) fn sanitize_terminal_text(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character == '\n' || character == '\t' || !character.is_control() {
                character
            } else {
                '\u{fffd}'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_controls_are_neutralized() {
        assert_eq!(
            sanitize_terminal_text("safe\n\u{1b}[31mred\u{9b}2J"),
            "safe\n�[31mred�2J"
        );
    }

    #[test]
    fn escaped_json_keeps_its_value() {
        let original = serde_json::to_string("before\u{9b}after")
            .unwrap_or_else(|error| unreachable!("JSON: {error}"));
        let escaped = escape_json_terminal_controls(&original);
        assert!(!escaped.contains('\u{9b}'));
        assert_eq!(
            serde_json::from_str::<String>(&escaped).ok().as_deref(),
            Some("before\u{9b}after")
        );
    }
}
