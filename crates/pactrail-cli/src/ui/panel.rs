//! Rounded, width-safe cards used by the startup dashboard and reviews.
//!
//! Cards are deliberately presentation-only: every line is clipped to the
//! requested cell width, and untrusted text has already been sanitized by the
//! theme before it reaches this module.
use crate::terminal::{strip_styles, truncate, width};
use crate::theme::Theme;

const MAX_CARD_WIDTH: usize = 96;
const MIN_CARD_WIDTH: usize = 24;

/// Total card width including the two-cell left margin.
pub(crate) fn card_width(columns: usize) -> usize {
    let available = columns.saturating_sub(2);
    // On terminals narrower than the preferred minimum, shrink with the
    // viewport instead of overflowing it.
    available.clamp(available.min(MIN_CARD_WIDTH), MAX_CARD_WIDTH)
}

/// Usable width for card body content.
pub(crate) fn body_width(columns: usize) -> usize {
    card_width(columns).saturating_sub(6)
}

/// Render a rounded card with a title, optional inline metadata, and body rows.
pub(crate) fn card(
    theme: &Theme,
    columns: usize,
    title: &str,
    meta: &str,
    body: Vec<String>,
) -> Vec<String> {
    let separator = if crate::ui::glyphs::ascii_requested() {
        "."
    } else {
        "·"
    };
    let label_plain = if meta.is_empty() {
        title.to_owned()
    } else {
        format!("{title} {separator} {meta}")
    };
    let label = if meta.is_empty() {
        theme.heading(title)
    } else {
        format!(
            "{} {}",
            theme.heading(title),
            theme.muted(&format!("{separator} {meta}"))
        )
    };
    card_styled(theme, columns, label, &label_plain, body)
}

/// Render a card whose title already carries its own semantic styling.
pub(crate) fn card_styled(
    theme: &Theme,
    columns: usize,
    label: String,
    label_plain: &str,
    body: Vec<String>,
) -> Vec<String> {
    let ascii = crate::ui::glyphs::ascii_requested();
    let (top_left, top_right, bottom_left, bottom_right, horizontal, vertical) = if ascii {
        ('+', '+', '+', '+', '-', '|')
    } else {
        ('╭', '╮', '╰', '╯', '─', '│')
    };
    let outer = card_width(columns);
    let body_width = body_width(columns);
    let label_max = outer.saturating_sub(8);
    let label = if width(label_plain) > label_max {
        theme.heading(&truncate(label_plain, label_max))
    } else {
        label
    };
    let label_cells = width(&strip_styles(&label));
    let rule = outer.saturating_sub(7 + label_cells).max(1);
    let mut lines = Vec::with_capacity(body.len() + 2);
    lines.push(format!(
        "  {} {label} {}{}",
        theme.muted(&format!("{top_left}{horizontal}")),
        theme.muted(&horizontal.to_string().repeat(rule)),
        theme.muted(&top_right.to_string())
    ));
    let rail = theme.muted(&vertical.to_string());
    for row in body {
        let plain = strip_styles(&row);
        let row = if width(&plain) > body_width {
            truncate(&plain, body_width)
        } else {
            row
        };
        let padding = body_width.saturating_sub(width(&strip_styles(&row)));
        lines.push(format!("  {rail} {row}{} {rail}", " ".repeat(padding)));
    }
    lines.push(format!(
        "  {}{}{}",
        theme.muted(&bottom_left.to_string()),
        theme.muted(&horizontal.to_string().repeat(outer.saturating_sub(4))),
        theme.muted(&bottom_right.to_string())
    ));
    lines
}

/// A card key/value row that wraps without leaving the card body.
pub(crate) fn field(
    theme: &Theme,
    columns: usize,
    label: &str,
    value: &str,
    paint: impl Fn(&str) -> String,
) -> Vec<String> {
    let width = body_width(columns);
    let value_width = width.saturating_sub(12).max(1);
    crate::terminal::wrap(value, value_width)
        .into_iter()
        .enumerate()
        .map(|(index, line)| {
            format!(
                "{} {}",
                theme.muted(&format!("{:<10}", if index == 0 { label } else { "" })),
                paint(&line)
            )
        })
        .collect()
}

/// A quiet section label inside a card.
pub(crate) fn group(theme: &Theme, label: &str) -> String {
    theme.heading(label)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cards_fit_every_supported_width_and_preserve_unicode() {
        for columns in [32, 40, 60, 80, 100, 120, 160, 240] {
            let theme = Theme::plain();
            let rows = card(
                &theme,
                columns,
                "Pactrail 2.1.2",
                "verification-native",
                vec![
                    field(
                        &theme,
                        columns,
                        "workspace",
                        "src/日本語/ component",
                        str::to_owned,
                    )
                    .join("\n"),
                    group(&theme, "Next"),
                    "type a task and press Enter".to_owned(),
                ],
            );
            assert!(
                rows.iter().all(|line| width(line) <= columns),
                "{columns}: {rows:#?}"
            );
            assert!(rows.join("").contains("日本語"));
        }
    }

    #[test]
    fn cards_shrink_below_the_preferred_minimum_instead_of_overflowing() {
        for columns in [12, 20, 24, 26, 32] {
            let theme = Theme::plain();
            let rows = card(
                &theme,
                columns,
                "Pactrail",
                "status",
                vec![
                    field(&theme, columns, "model", "fixture/model", str::to_owned).join("\n"),
                    "  next action".to_owned(),
                ],
            );
            assert!(
                rows.iter().all(|line| width(line) <= columns),
                "{columns}: {rows:#?}"
            );
        }
    }

    #[test]
    fn an_overlong_card_row_is_clipped_to_the_body() {
        let theme = Theme::plain();
        let rows = card(&theme, 40, "Title", "", vec!["x".repeat(400)]);
        assert!(rows.iter().all(|line| width(line) <= 40));
    }
}
