use std::io::IsTerminal;

use anstyle::{AnsiColor, Style};

use crate::output::sanitize_terminal_text;

#[derive(Clone)]
pub(crate) struct Theme {
    color: bool,
}

impl Theme {
    #[must_use]
    pub fn detect() -> Self {
        let color = std::io::stdout().is_terminal()
            && std::env::var_os("NO_COLOR").is_none()
            && std::env::var("TERM").map_or(true, |term| term != "dumb");
        Self { color }
    }

    #[cfg(test)]
    #[must_use]
    pub const fn plain() -> Self {
        Self { color: false }
    }

    #[must_use]
    pub fn brand(&self, value: &str) -> String {
        self.paint(
            Style::new().fg_color(Some(AnsiColor::Cyan.into())).bold(),
            value,
        )
    }

    #[must_use]
    pub fn heading(&self, value: &str) -> String {
        self.paint(
            Style::new()
                .fg_color(Some(AnsiColor::BrightCyan.into()))
                .bold(),
            value,
        )
    }

    #[must_use]
    pub fn success(&self, value: &str) -> String {
        self.paint(
            Style::new()
                .fg_color(Some(AnsiColor::BrightGreen.into()))
                .bold(),
            value,
        )
    }

    #[must_use]
    pub fn warning(&self, value: &str) -> String {
        self.paint(
            Style::new()
                .fg_color(Some(AnsiColor::BrightYellow.into()))
                .bold(),
            value,
        )
    }

    #[must_use]
    pub fn danger(&self, value: &str) -> String {
        self.paint(
            Style::new()
                .fg_color(Some(AnsiColor::BrightRed.into()))
                .bold(),
            value,
        )
    }

    #[must_use]
    pub fn muted(&self, value: &str) -> String {
        self.paint(
            Style::new().fg_color(Some(AnsiColor::BrightBlack.into())),
            value,
        )
    }

    #[must_use]
    pub fn accent(&self, value: &str) -> String {
        self.paint(
            Style::new().fg_color(Some(AnsiColor::BrightMagenta.into())),
            value,
        )
    }

    #[must_use]
    pub fn addition(&self, value: &str) -> String {
        self.paint(Style::new().fg_color(Some(AnsiColor::Green.into())), value)
    }

    #[must_use]
    pub fn deletion(&self, value: &str) -> String {
        self.paint(Style::new().fg_color(Some(AnsiColor::Red.into())), value)
    }

    #[must_use]
    pub fn code(&self, value: &str) -> String {
        self.paint(
            Style::new().fg_color(Some(AnsiColor::BrightWhite.into())),
            value,
        )
    }

    #[must_use]
    pub fn text(&self, value: &str) -> String {
        self.paint(Style::new(), value)
    }

    #[must_use]
    pub const fn has_color(&self) -> bool {
        self.color
    }

    fn paint(&self, style: Style, value: &str) -> String {
        let value = sanitize_terminal_text(value);
        if self.color {
            format!("{style}{value}{style:#}")
        } else {
            value
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn themed_untrusted_text_never_preserves_controls() {
        let theme = Theme::plain();
        assert_eq!(theme.success("ok\u{1b}[2J"), "ok\u{fffd}[2J");
    }
}
