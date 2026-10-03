//! The engine publishes facts; only the terminal owner reads input and prints them.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use pactrail_core::{ApprovalDecision, ApprovalRequest};
use tokio_util::sync::CancellationToken;

pub(crate) enum Message {
    Approval(Box<ApprovalRequest>, mpsc::SyncSender<ApprovalDecision>),
    Finished,
}

#[derive(Clone)]
pub(crate) struct Sink {
    sender: mpsc::SyncSender<Message>,
    pub wake: Arc<AtomicBool>,
    pub printer: reedline::ExternalPrinter<String>,
    operation: Arc<Mutex<String>>,
    cancellation: CancellationToken,
}

impl Sink {
    pub fn channel(cancellation: CancellationToken) -> (Self, mpsc::Receiver<Message>) {
        let (sender, receiver) = mpsc::sync_channel(128);
        (
            Self {
                sender,
                wake: Arc::new(AtomicBool::new(false)),
                printer: reedline::ExternalPrinter::new(128),
                operation: Arc::new(Mutex::new("Starting isolated transaction".to_owned())),
                cancellation,
            },
            receiver,
        )
    }
    pub fn publish(&self, message: Message) {
        self.wake.store(true, Ordering::Release);
        let _ = self.sender.send(message);
        self.wake.store(true, Ordering::Release);
    }
    pub fn operation(&self) -> String {
        self.operation
            .lock()
            .map_or_else(|_| "Operation not reported".to_owned(), |text| text.clone())
    }
    pub fn set_operation(&self, text: String) {
        if let Ok(mut operation) = self.operation.lock() {
            *operation = text;
        }
    }
    pub fn rows(&self, rows: &[String]) {
        let mut text = rows.join("\n");
        loop {
            if self.cancellation.is_cancelled() {
                let _ = self.printer.sender().try_send(text);
                return;
            }
            match self
                .printer
                .sender()
                .send_timeout(text, Duration::from_millis(100))
            {
                Ok(()) => return,
                Err(error) => {
                    if error.is_disconnected() {
                        return;
                    }
                    text = error.into_inner();
                }
            }
        }
    }
    pub fn approve(&self, request: ApprovalRequest) -> ApprovalDecision {
        let (sender, receiver) = mpsc::sync_channel(1);
        self.publish(Message::Approval(Box::new(request), sender));
        loop {
            if self.cancellation.is_cancelled() {
                return ApprovalDecision::Deny;
            }
            match receiver.recv_timeout(Duration::from_millis(100)) {
                Ok(decision) => return decision,
                Err(mpsc::RecvTimeoutError::Disconnected) => return ApprovalDecision::Deny,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        }
    }
}

pub(crate) fn supervise<F>(
    execution: F,
    terminal: Sink,
    cancellation: CancellationToken,
) -> tokio::task::JoinHandle<Result<crate::commands::CompletedRun, crate::commands::CliError>>
where
    F: Future<Output = Result<crate::commands::CompletedRun, crate::commands::CliError>>
        + Send
        + 'static,
{
    let mut engine = tokio::spawn(Box::pin(execution));
    tokio::spawn(async move {
        let result = tokio::select! {
            result = &mut engine => result,
            signal = tokio::signal::ctrl_c() => {
                if let Err(error) = signal { terminal.rows(&[format!("Ctrl-C listener failed: {error}; stopping safely")]); }
                cancellation.cancel();
                (&mut engine).await
            }
        };
        terminal.publish(Message::Finished);
        result.unwrap_or_else(|error| Err(crate::commands::CliError::Argument(format!("Engine task ended unexpectedly: {error}. Preserve .pactrail and inspect retained state."))))
    })
}

pub(crate) struct RunningPrompt<'a> {
    pub terminal: &'a Sink,
    pub started: std::time::Instant,
    pub backend: crate::cli::ProcessBackendArg,
}

impl reedline::Prompt for RunningPrompt<'_> {
    fn get_prompt_color(&self) -> reedline::Color {
        reedline::Color::Cyan
    }
    fn get_indicator_color(&self) -> reedline::Color {
        reedline::Color::Cyan
    }
    fn render_prompt_left(&self) -> std::borrow::Cow<'_, str> {
        let columns = crate::interactive::terminal_columns();
        let operation =
            crate::terminal::truncate(&self.terminal.operation(), columns.saturating_sub(10));
        let elapsed = u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let mode = match self.backend {
            crate::cli::ProcessBackendArg::Disabled => "commands: none",
            crate::cli::ProcessBackendArg::Native => "commands: host",
            crate::cli::ProcessBackendArg::Oci => "commands: sandbox",
        };
        let separator = if crate::ui::glyphs::ascii_requested() {
            "."
        } else {
            "·"
        };
        std::borrow::Cow::Owned(format!(
            "Running {separator} {operation}\n{mode} {separator} {} elapsed; Enter retains a draft\npactrail",
            crate::interactive::trace_duration(elapsed)
        ))
    }
    fn render_prompt_right(&self) -> std::borrow::Cow<'_, str> {
        std::borrow::Cow::Borrowed("")
    }
    fn render_prompt_indicator(&self, mode: reedline::PromptEditMode) -> std::borrow::Cow<'_, str> {
        std::borrow::Cow::Borrowed(crate::interactive::prompt_indicator(&mode))
    }
    fn render_prompt_multiline_indicator(&self) -> std::borrow::Cow<'_, str> {
        std::borrow::Cow::Borrowed(if crate::ui::glyphs::ascii_requested() {
            " | "
        } else {
            " │ "
        })
    }
    fn render_prompt_history_search_indicator(
        &self,
        search: reedline::PromptHistorySearch,
    ) -> std::borrow::Cow<'_, str> {
        std::borrow::Cow::Owned(format!(
            "(search: {}) ",
            crate::output::sanitize_terminal_text(&search.term)
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_closed_terminal_cannot_block_publication() {
        let (sink, receiver) = Sink::channel(CancellationToken::new());
        drop(receiver);
        sink.publish(Message::Finished);
        sink.set_operation("no terminal".to_owned());
        assert!(sink.wake.load(Ordering::Acquire));
    }
    #[test]
    fn cancelled_backpressure_releases_the_writer() {
        let cancellation = CancellationToken::new();
        let (sink, _receiver) = Sink::channel(cancellation.clone());
        for index in 0..128 {
            sink.rows(&[index.to_string()]);
        }
        cancellation.cancel();
        sink.rows(&["cannot block cleanup".to_owned()]);
        for index in 0..128 {
            assert_eq!(
                sink.printer.receiver().try_recv().ok().as_deref(),
                Some(index.to_string().as_str())
            );
        }
    }

    #[test]
    fn disconnected_approval_is_denied() {
        let (sink, receiver) = Sink::channel(CancellationToken::new());
        drop(receiver);
        let request = ApprovalRequest {
            binding: pactrail_core::ApprovalBinding {
                run_id: pactrail_core::RunId::new(),
                capability: pactrail_core::Capability::ProcessSpawn,
                resource: "fixture".to_owned(),
                actor_fingerprint: "fixture".to_owned(),
                backend_kind: "native".to_owned(),
                backend_identity: None,
                profile_digest: "fixture".to_owned(),
            },
            reason: "explicit consent required".to_owned(),
            presentation: std::collections::BTreeMap::new(),
        };
        assert_eq!(sink.approve(request), ApprovalDecision::Deny);
    }
}
