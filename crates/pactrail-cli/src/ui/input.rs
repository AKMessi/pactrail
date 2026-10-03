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

/// Canonical line input with a cancellable readiness wait; never create a second reader.
#[cfg(unix)]
pub(crate) fn read_terminal_line() -> io::Result<Signal> {
    use nix::poll::{PollFd, PollFlags, poll};
    use std::os::fd::AsFd;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    let mut interrupts = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
    let interrupted = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&interrupted);
    let listener = tokio::spawn(async move {
        if interrupts.recv().await.is_some() {
            flag.store(true, Ordering::Release);
        }
    });
    let stdin = io::stdin();
    let result = (|| {
        let mut descriptors = [PollFd::new(stdin.as_fd(), PollFlags::POLLIN)];
        loop {
            if interrupted.load(Ordering::Acquire) {
                return Ok(Signal::CtrlC);
            }
            match poll(&mut descriptors, 50_u16) {
                Ok(0) | Err(nix::errno::Errno::EINTR) => {}
                Err(error) => return Err(io::Error::other(error)),
                Ok(_) => {
                    if interrupted.load(Ordering::Acquire) {
                        return Ok(Signal::CtrlC);
                    }
                    return read_line(stdin.lock());
                }
            }
        }
    })();
    listener.abort();
    result
}

#[cfg(not(unix))]
pub(crate) fn read_terminal_line() -> io::Result<Signal> {
    read_line(io::stdin().lock())
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
