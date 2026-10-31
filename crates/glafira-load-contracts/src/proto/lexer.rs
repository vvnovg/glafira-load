//! Лексер подмножества proto3. Никогда не отказывает: незнакомый символ становится
//! `Tok::Punct`, а проблемы (незакрытая строка) уходят в предупреждения со строкой.

#[derive(Debug, Clone, PartialEq)]
pub(super) enum Tok {
    Ident(String),
    Str(String),
    Num(String),
    Punct(char),
}

pub(super) struct Token {
    pub(super) tok: Tok,
    pub(super) line: u32,
}

/// Токены файла и предупреждения лексера `(строка, текст)`.
pub(super) fn lex(src: &str) -> (Vec<Token>, Vec<(u32, String)>) {
    let b = src.as_bytes();
    let (mut line, mut out, mut warnings) = (1u32, Vec::new(), Vec::new());
    // UTF-8 BOM в начале файла не является токеном.
    let mut i = if src.starts_with('\u{feff}') { 3 } else { 0 };
    while i < b.len() {
        let c = b[i] as char;
        if c == '\n' {
            line += 1;
            i += 1;
        } else if c.is_ascii_whitespace() {
            i += 1;
        } else if src[i..].starts_with("//") {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
        } else if src[i..].starts_with("/*") {
            let end = src[i + 2..]
                .find("*/")
                .map(|e| i + 2 + e + 2)
                .unwrap_or(b.len());
            line += src[i..end].matches('\n').count() as u32;
            i = end;
        } else if c.is_ascii_alphabetic() || c == '_' || c == '.' {
            let start = i;
            while i < b.len()
                && ((b[i] as char).is_ascii_alphanumeric() || b[i] == b'_' || b[i] == b'.')
            {
                i += 1;
            }
            out.push(Token {
                tok: Tok::Ident(src[start..i].into()),
                line,
            });
        } else if c.is_ascii_digit() || c == '-' {
            let start = i;
            i += 1;
            while i < b.len() {
                let d = b[i];
                // Знак порядка: `1e+10`, `2E-3`.
                let exponent_sign =
                    (d == b'+' || d == b'-') && matches!(b[i - 1], b'e' | b'E') && i > start + 1;
                if (d as char).is_ascii_alphanumeric() || d == b'.' || exponent_sign {
                    i += 1;
                } else {
                    break;
                }
            }
            out.push(Token {
                tok: Tok::Num(src[start..i].into()),
                line,
            });
        } else if c == '"' || c == '\'' {
            let (start, start_line) = (i + 1, line);
            i += 1;
            while i < b.len() && b[i] as char != c {
                if b[i] == b'\\' {
                    i += 1;
                }
                if i < b.len() && b[i] == b'\n' {
                    line += 1;
                }
                i += 1;
            }
            if i >= b.len() {
                warnings.push((start_line, "unterminated string literal".to_string()));
            }
            out.push(Token {
                tok: Tok::Str(src[start..i.min(b.len())].into()),
                line: start_line,
            });
            i += 1;
        } else {
            // Любой другой символ (`/`, `|`, `@`, `+`, `:`, не-ASCII…) — пунктуация; разбор
            // пропускает её там, где она не нужна.
            let ch = src[i..].chars().next().unwrap_or(c);
            i += ch.len_utf8();
            if !ch.is_whitespace() {
                out.push(Token {
                    tok: Tok::Punct(ch),
                    line,
                });
            }
        }
    }
    (out, warnings)
}
