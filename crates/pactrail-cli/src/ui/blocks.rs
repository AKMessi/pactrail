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

pub(crate) fn field(
    theme: &Theme,
    columns: usize,
    label: &str,
    value: &str,
    paint: impl Fn(&str) -> String,
) -> Vec<String> {
    if columns < 60 {
        let mut lines = vec![format!("  {}", theme.muted(label))];
        lines.extend(
            wrap(value, columns.saturating_sub(4).clamp(1, 88))
                .into_iter()
                .map(|line| format!("    {}", paint(&line))),
        );
        return lines;
    }
    wrap(value, columns.saturating_sub(14).clamp(1, 88))
        .into_iter()
        .enumerate()
        .map(|(index, line)| {
            format!(
                "  {} {}",
                theme.muted(&format!("{:<10}", if index == 0 { label } else { "" })),
                paint(&line)
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ledger_blocks_preserve_long_unicode_data_and_fit_all_widths() {
        let value = "src/日本語/".to_owned() + &"component".repeat(30) + ".rs";
        for columns in [32, 40, 60, 80, 100, 120, 160] {
            let rows = field(&Theme::plain(), columns, "path", &value, |line| {
                line.to_owned()
            });
            assert!(
                rows.iter()
                    .all(|line| crate::terminal::width(line) <= columns)
            );
            assert!(rows.join("").contains("日本語"));
            assert!(
                rows.last()
                    .is_some_and(|line| std::path::Path::new(line).extension()
                        == Some(std::ffi::OsStr::new("rs")))
            );
            assert!(crate::terminal::width(&section(&Theme::plain(), columns, &value)) <= columns);
        }
    }
}
