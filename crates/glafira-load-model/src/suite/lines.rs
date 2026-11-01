//! Строка YAML по пути ключа для отчётов `check` (решение плана 1).
//!
//! Решение: повторный разбор в `serde_yaml_ng::Value` позиций не хранит, поэтому строка ищется
//! обходом текста по отступам: ключи блочных отображений (`key:`) и элементы блочных
//! последовательностей (`- `). Путь, уходящий внутрь flow-коллекции (`{…}`, `[…]`), блочного
//! скаляра или скаляра, как и путь с отсутствующим ключом или индексом, даёт строку ближайшего
//! найденного предка; `None` — только если не найден первый сегмент. Комментарии и пустые строки
//! пропускаются.

use crate::suite::expr::PathSeg;

#[derive(Debug, Clone)]
struct Line {
    no: u32,
    indent: usize,
    /// Текст без отступа.
    text: String,
}

fn lines(text: &str) -> Vec<Line> {
    text.lines()
        .enumerate()
        .filter_map(|(i, raw)| {
            let trimmed = raw.trim_start_matches(' ');
            let body = trimmed.trim_end();
            if body.is_empty() || body.starts_with('#') || body == "---" || body == "..." {
                return None;
            }
            Some(Line {
                no: i as u32 + 1,
                indent: raw.len() - trimmed.len(),
                text: body.to_string(),
            })
        })
        .collect()
}

/// Ключ в начале строки: `name:` / `"name":` / `'name':` с концом строки или пробелом после `:`.
/// Возвращает остаток после двоеточия.
fn key_rest<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    let rest = if let Some(q) = text.strip_prefix('"').or_else(|| text.strip_prefix('\'')) {
        let quote = text.as_bytes()[0] as char;
        let end = q.find(quote)?;
        if &q[..end] != key {
            return None;
        }
        &q[end + 1..]
    } else {
        text.strip_prefix(key)?
    };
    let rest = rest.strip_prefix(':')?;
    if rest.is_empty() || rest.starts_with(' ') {
        Some(rest.trim())
    } else {
        None
    }
}

/// Строки узла, начинающегося с `block[0]`: дочерние — с отступом больше, а у отображения ещё и
/// элементы `- ` с тем же отступом (YAML разрешает список на уровне ключа).
fn children(block: &[Line], at: usize, allow_items: bool) -> &[Line] {
    let ind = block[at].indent;
    let mut end = at + 1;
    while end < block.len()
        && (block[end].indent > ind
            || (allow_items && block[end].indent == ind && block[end].text.starts_with('-')))
    {
        end += 1;
    }
    &block[at + 1..end]
}

fn find(block: &[Line], path: &[PathSeg]) -> Option<u32> {
    let first = block.first()?;
    let Some(seg) = path.first() else {
        return Some(first.no);
    };
    let ind = first.indent;
    match seg {
        PathSeg::Key(key) => {
            for (i, line) in block.iter().enumerate() {
                if line.indent != ind || line.text.starts_with("- ") || line.text == "-" {
                    continue;
                }
                let Some(rest) = key_rest(&line.text, key) else {
                    continue;
                };
                // Значение в той же строке (скаляр, flow-коллекция, `|`/`>`): глубже не идём.
                if !rest.is_empty() {
                    return Some(line.no);
                }
                let child = children(block, i, true);
                return find(child, &path[1..]).or(Some(line.no));
            }
            None
        }
        PathSeg::Index(n) => {
            let mut seen = 0usize;
            for (i, line) in block.iter().enumerate() {
                if line.indent != ind || !(line.text == "-" || line.text.starts_with("- ")) {
                    continue;
                }
                if seen != *n {
                    seen += 1;
                    continue;
                }
                let content = line.text[1..].trim_start_matches(' ');
                let offset = line.text.len() - content.len();
                let rest = children(block, i, false);
                if content.is_empty() {
                    return find(rest, &path[1..]).or(Some(line.no));
                }
                // Элемент с содержимым на строке дефиса: отображение с отступом после `- `.
                let mut item = Vec::with_capacity(rest.len() + 1);
                item.push(Line {
                    no: line.no,
                    indent: ind + offset,
                    text: content.to_string(),
                });
                item.extend(rest.iter().cloned());
                return find(&item, &path[1..]).or(Some(line.no));
            }
            None
        }
    }
}

/// Строка (с 1) узла по пути или ближайшего найденного предка; `None`, если не найден даже
/// первый сегмент.
pub fn line_of(text: &str, path: &[PathSeg]) -> Option<u32> {
    find(&lines(text), path)
}

/// Путь в виде `scenarios[0].steps[1].http.body.items[0]` — ключ карты строк для IPC.
pub fn path_string(path: &[PathSeg]) -> String {
    let mut out = String::new();
    for seg in path {
        match seg {
            PathSeg::Key(k) => {
                if !out.is_empty() {
                    out.push('.');
                }
                out.push_str(k);
            }
            PathSeg::Index(i) => out.push_str(&format!("[{i}]")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use PathSeg::{Index, Key};

    fn k(s: &str) -> PathSeg {
        Key(s.into())
    }

    const SAMPLE: &str = include_str!("../../../../dev/fixtures/suites/orders/suite.yaml");

    #[test]
    fn finds_keys_items_and_nested_nodes_in_the_sample() {
        assert_eq!(line_of(SAMPLE, &[k("format")]), Some(1));
        assert_eq!(line_of(SAMPLE, &[k("data"), k("skus")]), Some(6));
        assert_eq!(
            line_of(SAMPLE, &[k("data"), k("users"), k("mode")]),
            Some(5),
            "flow: строка ключа"
        );
        assert_eq!(
            line_of(
                SAMPLE,
                &[k("defaults"), k("http"), k("headers"), k("Accept")]
            ),
            Some(8)
        );
        assert_eq!(line_of(SAMPLE, &[k("scenarios"), Index(0)]), Some(13));
        assert_eq!(
            line_of(SAMPLE, &[k("scenarios"), Index(0), k("drives")]),
            Some(14)
        );
        assert_eq!(
            line_of(
                SAMPLE,
                &[
                    k("scenarios"),
                    Index(0),
                    k("steps"),
                    Index(0),
                    k("http"),
                    k("operation")
                ]
            ),
            Some(19)
        );
        assert_eq!(
            line_of(
                SAMPLE,
                &[
                    k("scenarios"),
                    Index(0),
                    k("steps"),
                    Index(0),
                    k("http"),
                    k("body"),
                    k("items"),
                    Index(0),
                    k("qty")
                ]
            ),
            Some(21),
            "внутрь flow не идём"
        );
        assert_eq!(
            line_of(
                SAMPLE,
                &[
                    k("scenarios"),
                    Index(0),
                    k("steps"),
                    Index(0),
                    k("expect"),
                    k("jsonpath"),
                    k("$.id")
                ]
            ),
            Some(22)
        );
        assert_eq!(
            line_of(
                SAMPLE,
                &[
                    k("scenarios"),
                    Index(0),
                    k("steps"),
                    Index(1),
                    k("http"),
                    k("path"),
                    k("id")
                ]
            ),
            Some(25)
        );
        assert_eq!(
            line_of(SAMPLE, &[k("scenarios"), Index(0), k("steps"), Index(2)]),
            Some(27)
        );
        assert_eq!(
            line_of(
                SAMPLE,
                &[
                    k("scenarios"),
                    Index(0),
                    k("steps"),
                    Index(2),
                    k("kafka_consume"),
                    k("match"),
                    k("key")
                ]
            ),
            Some(29)
        );
        assert_eq!(
            line_of(
                SAMPLE,
                &[
                    k("scenarios"),
                    Index(0),
                    k("steps"),
                    Index(3),
                    k("kafka_produce"),
                    k("encoding")
                ]
            ),
            Some(35)
        );
        assert_eq!(
            line_of(SAMPLE, &[k("secrets"), Index(0)]),
            Some(3),
            "flow-список: строка ключа"
        );
        assert_eq!(
            line_of(SAMPLE, &[k("scenarios"), Index(1)]),
            Some(12),
            "нет элемента: строка предка"
        );
        assert_eq!(line_of(SAMPLE, &[k("nope")]), None);
        assert_eq!(
            line_of(
                SAMPLE,
                &[k("scenarios"), Index(0), k("steps"), Index(0), k("nope")]
            ),
            Some(17),
            "неизвестный ключ: строка предка"
        );
        assert_eq!(line_of(SAMPLE, &[]), Some(1));
    }

    #[test]
    fn handles_quoted_keys_lists_at_key_level_and_block_scalars() {
        let text = "a:\n- x: 1\n  'q': 2\n-\n  y: 3\nb: |\n  k: v\n  z: 9\n\"c\": 4\n";
        assert_eq!(line_of(text, &[k("a"), Index(0), k("q")]), Some(3));
        assert_eq!(line_of(text, &[k("a"), Index(1), k("y")]), Some(5));
        assert_eq!(
            line_of(text, &[k("b"), k("k")]),
            Some(6),
            "блочный скаляр не разбирается"
        );
        assert_eq!(line_of(text, &[k("c")]), Some(9));
        assert_eq!(
            line_of(text, &[k("z")]),
            None,
            "строки скаляра не ключи верхнего уровня"
        );
        assert_eq!(line_of("", &[k("a")]), None);
    }

    #[test]
    fn path_string_formats_keys_and_indexes() {
        assert_eq!(
            path_string(&[k("scenarios"), Index(0), k("steps"), Index(1), k("http")]),
            "scenarios[0].steps[1].http"
        );
        assert_eq!(path_string(&[]), "");
    }
}
