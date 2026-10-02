//! Line input for terminals that cannot support a cursor-based editor.
use std::io::{self, BufRead, IsTerminal};

use reedline::Signal;

pub(crate) fn plain_requested() -> bool {
    std::env::var("PACTRAIL_PLAIN").is_ok_and(|value| value == "1")
        || std::env::var("TERM").is_ok_and(|value| value == "dumb")
        || !io::stdin().is_terminal()
}

pub(crate) fn read_line(reader: impl BufRead) -> io::Result<Signal> {
    let mut text = String::new();
    let bytes = reader
        .take(crate::terminal::MAX_DRAFT_BYTES + 1)
        .read_line(&mut text)?;
    if bytes == 0 {
        return Ok(Signal::CtrlD);
    }
    if bytes as u64 > crate::terminal::MAX_DRAFT_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "input exceeds 64 KiB; use a smaller task",
        ));
    }
    Ok(Signal::Success(
        text.trim_end_matches(['\r', '\n']).to_owned(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn plain_input_preserves_unicode_and_cannot_accept_an_oversized_prefix() {
        assert!(matches!(read_line(&b""[..]), Ok(Signal::CtrlD)));
        assert!(
            matches!(read_line("日本語 task\r\n".as_bytes()), Ok(Signal::Success(text)) if text == "日本語 task")
        );
        assert!(read_line(vec![b'x'; 65_537].as_slice()).is_err());
    }
}
