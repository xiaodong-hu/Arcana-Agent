use ratatui::prelude::*;
use tree_sitter_highlight::{HighlightConfiguration, HighlightEvent, Highlighter};

/// Recognized highlight capture names (order matters — index maps to color).
const HIGHLIGHT_NAMES: &[&str] = &[
    "attribute",
    "comment",
    "constant",
    "constant.builtin",
    "constructor",
    "embedded",
    "function",
    "function.builtin",
    "keyword",
    "module",
    "number",
    "operator",
    "property",
    "property.builtin",
    "punctuation",
    "punctuation.bracket",
    "punctuation.delimiter",
    "punctuation.special",
    "string",
    "string.special",
    "tag",
    "type",
    "type.builtin",
    "variable",
    "variable.builtin",
    "variable.parameter",
];

/// Map highlight index to foreground color.
fn highlight_color(idx: usize) -> Color {
    match HIGHLIGHT_NAMES.get(idx) {
        Some(&"keyword") => Color::Rgb(198, 120, 221),        // purple
        Some(&"string") | Some(&"string.special") => Color::Rgb(152, 195, 121), // green
        Some(&"comment") => Color::Rgb(92, 99, 112),          // gray
        Some(&"function") | Some(&"function.builtin") => Color::Rgb(97, 175, 239), // blue
        Some(&"type") | Some(&"type.builtin") => Color::Rgb(229, 192, 123), // yellow
        Some(&"number") | Some(&"constant") | Some(&"constant.builtin") => Color::Rgb(209, 154, 102), // orange
        Some(&"operator") => Color::Rgb(86, 182, 194),        // cyan
        Some(&"variable.builtin") | Some(&"property.builtin") => Color::Rgb(224, 108, 117), // red
        Some(&"variable") => Color::Rgb(224, 108, 117),       // red
        Some(&"variable.parameter") => Color::Rgb(171, 178, 191), // light gray
        Some(&"attribute") => Color::Rgb(229, 192, 123),      // yellow
        Some(&"constructor") => Color::Rgb(229, 192, 123),    // yellow
        Some(&"module") => Color::Rgb(97, 175, 239),          // blue
        Some(&"tag") => Color::Rgb(224, 108, 117),            // red
        Some(&"property") => Color::Rgb(224, 108, 117),       // red
        Some(&"punctuation") | Some(&"punctuation.bracket") | Some(&"punctuation.delimiter") | Some(&"punctuation.special") => {
            Color::Rgb(171, 178, 191) // light gray
        }
        Some(&"embedded") => Color::Rgb(198, 120, 221),       // purple
        _ => Color::Rgb(171, 178, 191),                        // default: light gray
    }
}

/// Detect language from file extension.
pub fn detect_language(path: &str) -> Option<&'static str> {
    let ext = path.rsplit('.').next()?;
    match ext {
        "rs" => Some("rust"),
        "py" | "pyi" => Some("python"),
        "js" | "mjs" | "cjs" | "jsx" => Some("javascript"),
        "ts" | "mts" | "cts" => Some("typescript"),
        "tsx" => Some("tsx"),
        "toml" => Some("toml"),
        "json" => Some("json"),
        "sh" | "bash" | "zsh" => Some("bash"),
        _ => None,
    }
}

/// Normalize Markdown fence language names to tree-sitter grammars that are
/// currently compiled into Arcana.
pub fn normalize_language(lang: &str) -> Option<&'static str> {
    let lang = lang.trim().trim_start_matches('.').to_ascii_lowercase();
    let lang = lang.split_whitespace().next().unwrap_or("");
    match lang {
        "rs" | "rust" => Some("rust"),
        "py" | "pyi" | "python" | "python3" => Some("python"),
        "js" | "mjs" | "cjs" | "jsx" | "javascript" => Some("javascript"),
        "ts" | "mts" | "cts" | "typescript" => Some("typescript"),
        "tsx" => Some("tsx"),
        "toml" => Some("toml"),
        "json" | "jsonc" => Some("json"),
        "sh" | "bash" | "zsh" | "shell" => Some("bash"),
        _ => None,
    }
}

/// Build a HighlightConfiguration for the given language name.
fn build_config(lang: &str) -> Option<HighlightConfiguration> {
    let mut config = match lang {
        "rust" => HighlightConfiguration::new(
            tree_sitter_rust::LANGUAGE.into(),
            "rust",
            tree_sitter_rust::HIGHLIGHTS_QUERY,
            tree_sitter_rust::INJECTIONS_QUERY,
            "",
        ).ok()?,
        "python" => HighlightConfiguration::new(
            tree_sitter_python::LANGUAGE.into(),
            "python",
            tree_sitter_python::HIGHLIGHTS_QUERY,
            "",
            "",
        ).ok()?,
        "javascript" => HighlightConfiguration::new(
            tree_sitter_javascript::LANGUAGE.into(),
            "javascript",
            tree_sitter_javascript::HIGHLIGHT_QUERY,
            tree_sitter_javascript::INJECTIONS_QUERY,
            tree_sitter_javascript::LOCALS_QUERY,
        ).ok()?,
        "typescript" => HighlightConfiguration::new(
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            "typescript",
            tree_sitter_typescript::HIGHLIGHTS_QUERY,
            tree_sitter_typescript::LOCALS_QUERY,
            "",
        ).ok()?,
        "tsx" => HighlightConfiguration::new(
            tree_sitter_typescript::LANGUAGE_TSX.into(),
            "tsx",
            tree_sitter_typescript::HIGHLIGHTS_QUERY,
            tree_sitter_typescript::LOCALS_QUERY,
            "",
        ).ok()?,
        "toml" => HighlightConfiguration::new(
            tree_sitter_toml_ng::LANGUAGE.into(),
            "toml",
            tree_sitter_toml_ng::HIGHLIGHTS_QUERY,
            "",
            "",
        ).ok()?,
        "json" => HighlightConfiguration::new(
            tree_sitter_json::LANGUAGE.into(),
            "json",
            tree_sitter_json::HIGHLIGHTS_QUERY,
            "",
            "",
        ).ok()?,
        "bash" => HighlightConfiguration::new(
            tree_sitter_bash::LANGUAGE.into(),
            "bash",
            tree_sitter_bash::HIGHLIGHT_QUERY,
            "",
            "",
        ).ok()?,
        _ => return None,
    };
    config.configure(HIGHLIGHT_NAMES);
    Some(config)
}

/// A styled span: text + foreground color.
#[derive(Debug, Clone)]
pub struct StyledSpan {
    pub text: String,
    pub fg: Color,
}

/// Highlight source code, returning styled spans per line.
/// Falls back to plain white if language is unsupported or parsing fails.
pub fn highlight_lines(source: &str, lang: &str) -> Vec<Vec<StyledSpan>> {
    let default_fg = Color::Rgb(171, 178, 191);

    let config = match build_config(lang) {
        Some(c) => c,
        None => return plain_lines(source, default_fg),
    };

    let mut highlighter = Highlighter::new();
    let highlights = match highlighter.highlight(&config, source.as_bytes(), None, |_| None) {
        Ok(h) => h,
        Err(_) => return plain_lines(source, default_fg),
    };

    let mut result: Vec<Vec<StyledSpan>> = vec![Vec::new()];
    let mut current_fg = default_fg;

    for event in highlights {
        match event {
            Ok(HighlightEvent::Source { start, end }) => {
                let text = &source[start..end];
                // Split by newlines to assign spans to correct lines
                for (i, part) in text.split('\n').enumerate() {
                    if i > 0 {
                        result.push(Vec::new());
                    }
                    if !part.is_empty() {
                        result.last_mut().unwrap().push(StyledSpan {
                            text: part.to_string(),
                            fg: current_fg,
                        });
                    }
                }
            }
            Ok(HighlightEvent::HighlightStart(s)) => {
                current_fg = highlight_color(s.0);
            }
            Ok(HighlightEvent::HighlightEnd) => {
                current_fg = default_fg;
            }
            Err(_) => break,
        }
    }

    result
}

fn plain_lines(source: &str, fg: Color) -> Vec<Vec<StyledSpan>> {
    source.split('\n').map(|line| {
        if line.is_empty() {
            Vec::new()
        } else {
            vec![StyledSpan { text: line.to_string(), fg }]
        }
    }).collect()
}
