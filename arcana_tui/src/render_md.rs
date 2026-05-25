use ratatui::prelude::*;
use std::borrow::Cow;
use std::sync::OnceLock;
use syntect::easy::HighlightLines;
use syntect::highlighting::{Style as SynStyle, ThemeSet};
use syntect::parsing::SyntaxSet;

/// Blue color for inline code.
const CODE_BLUE: Color = Color::Rgb(0, 128, 255);
const DEFAULT_CODE_FG: Color = Color::Rgb(171, 178, 191);
const COMMENT_FG: Color = Color::Rgb(92, 99, 112);
const STRING_FG: Color = Color::Rgb(152, 195, 121);
const KEYWORD_FG: Color = Color::Rgb(198, 120, 221);
const NUMBER_FG: Color = Color::Rgb(209, 154, 102);

/// Cached syntax definitions — loaded once, reused across all renders.
fn syntax_set() -> &'static SyntaxSet {
    static SS: OnceLock<SyntaxSet> = OnceLock::new();
    SS.get_or_init(SyntaxSet::load_defaults_newlines)
}

fn syntax_theme() -> &'static syntect::highlighting::Theme {
    static TS: OnceLock<ThemeSet> = OnceLock::new();
    let ts = TS.get_or_init(ThemeSet::load_defaults);
    &ts.themes["base16-ocean.dark"]
}

/// Render a markdown text block into styled Lines.
/// - Inline `code` -> blue without backticks.
/// - Inline **bold** -> bold without stars.
/// - Fenced code blocks -> blank line before/after and syntax highlighted.
/// - Double+ newlines are compacted outside fenced code blocks.
pub fn render_markdown<'a>(text: &str, base_style: Style) -> Vec<Line<'a>> {
    let compacted = compact_newlines_preserving_code_blocks(text);
    let mut result: Vec<Line<'a>> = Vec::new();
    let mut lines_iter = compacted.lines().peekable();
    let ss = syntax_set();
    let theme = syntax_theme();

    while let Some(line) = lines_iter.next() {
        if let Some(lang) = fence_language(line) {
            push_blank_line(&mut result);

            let mut code = String::new();
            while let Some(code_line) = lines_iter.next() {
                if fence_language(code_line).is_some() {
                    break;
                }
                if !code.is_empty() {
                    code.push('\n');
                }
                code.push_str(code_line);
            }

            result.extend(highlight_code_block(&code, &lang, base_style, &ss, theme));
            push_blank_line(&mut result);
        } else {
            result.push(parse_inline_markup(line, base_style));
        }
    }

    result
}

fn fence_language(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    let rest = trimmed.strip_prefix("```")?;
    Some(rest.trim().to_string())
}

fn push_blank_line<'a>(lines: &mut Vec<Line<'a>>) {
    let is_already_blank = lines
        .last()
        .map(|line| line.spans.iter().all(|span| span.content.is_empty()))
        .unwrap_or(false);
    if !is_already_blank {
        lines.push(Line::from(""));
    }
}

fn highlight_code_block<'a>(
    code: &str,
    lang: &str,
    base_style: Style,
    ss: &SyntaxSet,
    theme: &syntect::highlighting::Theme,
) -> Vec<Line<'a>> {
    if let Some(tree_lang) = crate::highlight::normalize_language(lang) {
        return crate::highlight::highlight_lines(code, tree_lang)
            .into_iter()
            .map(|spans| {
                if spans.is_empty() {
                    Line::from("")
                } else {
                    Line::from(
                        spans
                            .into_iter()
                            .map(|span| {
                                Span::styled(
                                    span.text,
                                    Style::default()
                                        .fg(span.fg)
                                        .bg(base_style.bg.unwrap_or(Color::Reset)),
                                )
                            })
                            .collect::<Vec<_>>(),
                    )
                }
            })
            .collect();
    }

    if let Some(lines) = highlight_with_syntect(code, lang, ss, theme) {
        return lines;
    }

    highlight_generic(code, lang, base_style)
}

fn highlight_with_syntect<'a>(
    code: &str,
    lang: &str,
    ss: &SyntaxSet,
    theme: &syntect::highlighting::Theme,
) -> Option<Vec<Line<'a>>> {
    let syntax = ss
        .find_syntax_by_token(syntect_token(lang).as_ref())
        .or_else(|| ss.find_syntax_by_name(lang))?;
    let mut highlighter = HighlightLines::new(syntax, theme);
    let mut out = Vec::new();

    for line in code.split('\n') {
        match highlighter.highlight_line(line, ss) {
            Ok(ranges) => out.push(Line::from(
                ranges
                    .into_iter()
                    .map(|(style, text)| Span::styled(text.to_string(), syn_to_ratatui(style)))
                    .collect::<Vec<_>>(),
            )),
            Err(_) => return None,
        }
    }

    Some(out)
}

fn syntect_token(lang: &str) -> Cow<'_, str> {
    let normalized = lang.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "c++" | "cpp" | "cxx" | "cc" => Cow::Borrowed("cpp"),
        "c" | "h" => Cow::Borrowed("c"),
        "zig" => Cow::Borrowed("zig"),
        "jl" | "julia" => Cow::Borrowed("julia"),
        "tex" | "latex" | "ltx" => Cow::Borrowed("tex"),
        "typst" | "typ" => Cow::Borrowed("typst"),
        "md" | "markdown" => Cow::Borrowed("markdown"),
        "yaml" | "yml" => Cow::Borrowed("yaml"),
        "html" => Cow::Borrowed("html"),
        "css" => Cow::Borrowed("css"),
        "go" | "golang" => Cow::Borrowed("go"),
        "java" => Cow::Borrowed("java"),
        "php" => Cow::Borrowed("php"),
        "rb" | "ruby" => Cow::Borrowed("ruby"),
        "swift" => Cow::Borrowed("swift"),
        "scala" => Cow::Borrowed("scala"),
        _ => Cow::Owned(normalized),
    }
}

fn highlight_generic<'a>(code: &str, lang: &str, base_style: Style) -> Vec<Line<'a>> {
    code.split('\n')
        .map(|line| Line::from(generic_line_spans(line, lang, base_style)))
        .collect()
}

fn generic_line_spans<'a>(line: &str, lang: &str, base_style: Style) -> Vec<Span<'a>> {
    if line.trim_start().starts_with("//")
        || line.trim_start().starts_with('#')
        || line.trim_start().starts_with('%')
    {
        return vec![Span::styled(line.to_string(), base_style.fg(COMMENT_FG))];
    }

    let keywords = generic_keywords(lang);
    let mut spans = Vec::new();
    let mut rest = line;

    while !rest.is_empty() {
        if let Some((prefix, quote, after_quote)) = split_next_quote(rest) {
            push_generic_words(&mut spans, prefix, keywords, base_style);
            let Some(end) = after_quote.find(quote) else {
                spans.push(Span::styled(
                    format!("{quote}{after_quote}"),
                    base_style.fg(STRING_FG),
                ));
                return spans;
            };
            let string = &after_quote[..end];
            spans.push(Span::styled(
                format!("{quote}{string}{quote}"),
                base_style.fg(STRING_FG),
            ));
            rest = &after_quote[end + quote.len_utf8()..];
        } else {
            push_generic_words(&mut spans, rest, keywords, base_style);
            break;
        }
    }

    spans
}

fn split_next_quote(text: &str) -> Option<(&str, char, &str)> {
    let single = text.find('\'');
    let double = text.find('"');
    let idx = match (single, double) {
        (Some(a), Some(b)) => a.min(b),
        (Some(a), None) | (None, Some(a)) => a,
        (None, None) => return None,
    };
    let quote = text[idx..].chars().next()?;
    Some((&text[..idx], quote, &text[idx + quote.len_utf8()..]))
}

fn push_generic_words<'a>(
    spans: &mut Vec<Span<'a>>,
    text: &str,
    keywords: &[&str],
    base_style: Style,
) {
    for part in split_word_boundaries(text) {
        let style = if keywords.contains(&part) {
            base_style.fg(KEYWORD_FG).add_modifier(Modifier::BOLD)
        } else if part.chars().next().is_some_and(|c| c.is_ascii_digit()) {
            base_style.fg(NUMBER_FG)
        } else {
            base_style.fg(DEFAULT_CODE_FG)
        };
        spans.push(Span::styled(part.to_string(), style));
    }
}

fn split_word_boundaries(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut last_word = None;

    for (idx, ch) in text.char_indices() {
        let is_word = ch.is_ascii_alphanumeric() || ch == '_';
        if let Some(prev) = last_word {
            if prev != is_word {
                out.push(&text[start..idx]);
                start = idx;
            }
        }
        last_word = Some(is_word);
    }
    if start < text.len() {
        out.push(&text[start..]);
    }
    out
}

fn generic_keywords(lang: &str) -> &'static [&'static str] {
    match lang.trim().to_ascii_lowercase().as_str() {
        "typst" | "typ" => &[
            "let", "set", "show", "import", "include", "if", "else", "for", "in",
        ],
        "tex" | "latex" | "ltx" => &["begin", "end", "documentclass", "usepackage", "newcommand"],
        "julia" | "jl" => &[
            "function", "end", "if", "else", "elseif", "for", "while", "let", "local", "global",
            "struct", "module", "using", "import", "return",
        ],
        "zig" => &[
            "const", "var", "fn", "pub", "if", "else", "while", "for", "return", "struct",
        ],
        _ => &[
            "fn", "let", "const", "var", "if", "else", "for", "while", "return", "class", "struct",
            "import", "from", "use", "pub", "def",
        ],
    }
}

/// Parse a single line for inline `code` spans and **bold** spans.
fn parse_inline_markup<'a>(text: &str, base_style: Style) -> Line<'a> {
    let mut spans: Vec<Span<'a>> = Vec::new();
    let mut rest = text;

    while !rest.is_empty() {
        let code_pos = rest.find('`');
        let bold_pos = rest.find("**");
        let (start, delimiter) = match (code_pos, bold_pos) {
            (Some(c), Some(b)) if c < b => (c, "`"),
            (Some(_), Some(b)) => (b, "**"),
            (Some(c), None) => (c, "`"),
            (None, Some(b)) => (b, "**"),
            (None, None) => {
                spans.push(Span::styled(rest.to_string(), base_style));
                break;
            }
        };

        if start > 0 {
            spans.push(Span::styled(rest[..start].to_string(), base_style));
        }

        let after = &rest[start + delimiter.len()..];
        if let Some(end) = after.find(delimiter) {
            let content = &after[..end];
            let style = if delimiter == "`" {
                Style::default().fg(CODE_BLUE)
            } else {
                base_style.add_modifier(Modifier::BOLD)
            };
            spans.push(Span::styled(content.to_string(), style));
            rest = &after[end + delimiter.len()..];
        } else {
            spans.push(Span::styled(rest[start..].to_string(), base_style));
            break;
        }
    }

    if spans.is_empty() {
        spans.push(Span::styled(String::new(), base_style));
    }
    Line::from(spans)
}

/// Compact multiple consecutive newlines outside fenced code blocks.
fn compact_newlines_preserving_code_blocks(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut prev_was_empty = false;
    let mut in_code = false;

    for line in text.split('\n') {
        if fence_language(line).is_some() {
            if !result.is_empty() {
                result.push('\n');
            }
            result.push_str(line);
            in_code = !in_code;
            prev_was_empty = false;
            continue;
        }

        if in_code {
            if !result.is_empty() {
                result.push('\n');
            }
            result.push_str(line);
            continue;
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            prev_was_empty = true;
        } else {
            if prev_was_empty && (trimmed.starts_with('#') || trimmed.starts_with("---")) {
                result.push('\n');
            }
            prev_was_empty = false;
            if !result.is_empty() {
                result.push('\n');
            }
            result.push_str(line);
        }
    }
    result
}

/// Convert syntect Style to ratatui Style.
fn syn_to_ratatui(style: SynStyle) -> Style {
    let fg = Color::Rgb(style.foreground.r, style.foreground.g, style.foreground.b);
    Style::default().fg(fg)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line_text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<Vec<_>>()
            .join("")
    }

    #[test]
    fn adds_blank_lines_around_fenced_code() {
        let lines = render_markdown("before\n```python\nprint(1)\n```\nafter", Style::default());
        let text = lines.iter().map(line_text).collect::<Vec<_>>();

        assert_eq!(text[0], "before");
        assert_eq!(text[1], "");
        assert_eq!(text[2], "print(1)");
        assert_eq!(text[3], "");
        assert_eq!(text[4], "after");
    }

    #[test]
    fn parses_inline_code_and_bold() {
        let line = parse_inline_markup("use `code` and **bold**", Style::default());
        let text = line_text(&line);

        assert_eq!(text, "use code and bold");
        assert_eq!(line.spans[1].style.fg, Some(CODE_BLUE));
        assert!(line.spans[3].style.add_modifier.contains(Modifier::BOLD));
    }
}
