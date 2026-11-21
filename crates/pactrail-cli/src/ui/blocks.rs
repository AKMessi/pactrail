use crate::terminal::{truncate, wrap};
use crate::theme::Theme;

pub(crate) fn section(theme: &Theme, columns: usize, title: &str) -> String {
    format!(
        "  {}",
        theme.heading(&truncate(title, columns.saturating_sub(2)))
    )
}

pub(crate) fn note(theme: &Theme, columns: usize, text: &str) -> Vec<String> {
    wrap(text, columns.saturating_sub(4).clamp(1, 88))
        .into_iter()
        .map(|line| format!("    {}", theme.text(&line)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ledger_blocks_preserve_long_unicode_data_and_fit_all_widths() {
        let value = "src/日本語/".to_owned() + &"component".repeat(30) + ".rs";
        for columns in [32, 40, 60, 80, 100, 120, 160] {
            let rows = note(&Theme::plain(), columns, &value);
            assert!(
                rows.iter()
                    .all(|line| crate::terminal::width(line) <= columns)
            );
            assert!(rows.join("").contains("日本語"));
            assert!(rows.join("").contains("rs"));
            assert!(crate::terminal::width(&section(&Theme::plain(), columns, &value)) <= columns);
        }
    }
}
