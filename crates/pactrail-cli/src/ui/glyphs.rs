//! ASCII fallbacks affect app-owned decorations, never filenames or model prose.
pub(crate) fn ascii_requested() -> bool {
    if crate::ui::input::plain_requested() {
        return true;
    }
    if std::env::var("PACTRAIL_ASCII").is_ok_and(|value| value == "1") {
        return true;
    }
    ["LC_ALL", "LC_CTYPE", "LANG"]
        .iter()
        .find_map(|name| std::env::var(name).ok().filter(|value| !value.is_empty()))
        .is_some_and(|locale| matches!(locale.as_str(), "C" | "POSIX"))
}

pub(crate) fn symbol(value: &str, ascii: bool) -> &str {
    if !ascii {
        return value;
    }
    match value {
        "✓" => "[ok]",
        "×" => "[x]",
        "!" | "▲" => "[!]",
        "●" | "◉" => "*",
        "○" => "o",
        "■" => "[stop]",
        "◇" => "<>",
        "◆" => "+",
        "◈" => "#",
        "↻" => "[retry]",
        "│" => "|",
        "❯" => ">",
        "▣" => "[review]",
        _ => value,
    }
}

/// Only call with compile-time UI labels; untrusted text is never transliterated.
pub(crate) fn label(value: &'static str, ascii: bool) -> String {
    if !ascii {
        return value.to_owned();
    }
    let mut text = String::new();
    for c in value.chars() {
        text.push_str(match c {
            '✓' => "[ok]",
            '×' => "[x]",
            '▲' => "[!]",
            '●' | '◉' => "*",
            '○' => "o",
            '■' => "[stop]",
            '◇' => "<>",
            '◈' => "#",
            '↻' => "[retry]",
            '│' => "|",
            '─' | '━' | '−' => "-",
            '◆' | '╭' | '╰' => "+",
            '❯' => ">",
            '…' => "...",
            '·' => ".",
            '→' => "->",
            _ => {
                text.push(c);
                continue;
            }
        });
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ascii_states_are_distinct_and_unicode_prose_is_not_a_decoration() {
        assert_eq!(symbol("✓", true), "[ok]");
        assert_eq!(symbol("×", true), "[x]");
        assert_eq!(label("◇ Answered", true), "<> Answered");
        assert!(label("╭─ Execution trace · …", true).is_ascii());
        assert_eq!(symbol("src/日本語.rs", true), "src/日本語.rs");
    }
}
