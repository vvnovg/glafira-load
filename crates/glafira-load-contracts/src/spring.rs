//! Spring MVC/WebFlux и Spring Kafka по текстовым правилам, без компиляции (спека L1 §5.2):
//! единственная эвристика по коду в L1. Путь через константу — предупреждение, операция не создаётся.
//! Комментарии перед разбором затираются пробелами (смещения и номера строк сохраняются),
//! строковые литералы остаются: в них пути и топики.

use std::sync::LazyLock;

use glafira_load_model::registry::{Address, OpKind, OperationDraft, Source, SourceKind};
use glafira_load_model::schema::SchemaSummary;
use regex::Regex;

use crate::{ContractInput, ParseError, ParseOptions, ParseResult};

/// Объявление класса, а не слово «class» в тексте: только в начале строки после аннотаций
/// (в т. ч. на той же строке, `@RestController @RequestMapping("/api") class Foo`) и модификаторов.
/// Аннотация — `@Имя` с необязательными скобками (вложенные — на один уровень, как у `MAPPING`).
/// Позиция ключевого слова — группа `kw`: всё, что левее, относится к аннотациям класса.
static CLASS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?m)^\s*(?:(?:@[\w.]+(?:\((?:[^()]|\([^()]*\))*\))?|public|private|protected|internal|open|abstract|final|data|sealed|static|enum|annotation|inner|value|companion|fun)\s+)*(?P<kw>class|interface|object)\s+\w+",
    )
    .unwrap()
});
static MAPPING: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"@(Get|Post|Put|Delete|Patch|Request)Mapping\s*(?:\(((?:[^()]|\([^()]*\))*)\))?")
        .unwrap()
});
/// Именованный аргумент пути: `value = …` / `path = …` (значение — до конца элемента).
static ELEM_PATH: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)^(?:value|path)\s*=\s*(.*)$").unwrap());
/// Именованное имя параметра: `value = "x"` / `name = "x"`.
static ELEM_PARAM_NAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?s)^(?:value|name)\s*=\s*(.*)$"#).unwrap());
/// Прочие известные атрибуты маппинга, не задающие путь.
static ELEM_OTHER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?:method|consumes|produces|params|headers|name)\s*=").unwrap());
static ELEM_TOPICS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)^topics\s*=\s*(.*)$").unwrap());
static METHOD_ATTR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"RequestMethod\.(\w+)").unwrap());
static JAVA_METHOD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^\s*(?:public|private|protected|default)?\s*(?:static\s+|final\s+|synchronized\s+)*[\w<>\[\],.?\s]+?\s+(\w+)\s*\(").unwrap()
});
static KOTLIN_METHOD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\bfun\s+(?:<[^>]*>\s*)?(\w+)\s*\(").unwrap());
static BODY_JAVA: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"@RequestBody\s+(?:@\w+\s+)*(?:final\s+)?([\w.<>,? ]+?)\s+\w+\s*[,)]").unwrap()
});
static BODY_KOTLIN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"@RequestBody\s+\w+\s*:\s*([\w.<>,? ]+?)\s*[,)=]").unwrap());
static PATH_VAR_JAVA: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"@PathVariable(?:\s*\(\s*(?:value\s*=\s*|name\s*=\s*)?"(\w+)"[^)]*\))?\s+(?:final\s+)?[\w.<>]+\s+(\w+)"#).unwrap()
});
static PATH_VAR_KOTLIN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"@PathVariable(?:\s*\(\s*(?:value\s*=\s*|name\s*=\s*)?"(\w+)"[^)]*\))?\s+(\w+)\s*:"#,
    )
    .unwrap()
});
static QUERY_JAVA: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"@RequestParam(?:\s*\(([^)]*)\))?\s+(?:final\s+)?[\w.<>]+\s+(\w+)"#).unwrap()
});
static QUERY_KOTLIN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"@RequestParam(?:\s*\(([^)]*)\))?\s+(\w+)\s*:"#).unwrap());
static LISTENER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"@KafkaListener\s*\(((?:[^()]|\([^()]*\))*)\)").unwrap());
/// Начало вызова `send(` у шаблона Kafka; первый аргумент разбирается отдельно.
static SEND: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\w*(?:kafka|template)\w*\s*\.\s*send\s*\(").unwrap());

fn line_at(text: &str, pos: usize) -> u32 {
    text[..pos].matches('\n').count() as u32 + 1
}

/// Копия исходника, где содержимое комментариев (`//`, `/* */`, Javadoc/KDoc) заменено пробелами
/// той же длины; переводы строк и строковые литералы не трогаются, поэтому смещения и номера
/// строк совпадают с оригиналом. Вложенные `/* */` учитываются только в Kotlin.
fn mask_comments(text: &str, kotlin: bool) -> String {
    let b = text.as_bytes();
    let n = b.len();
    let mut out = b.to_vec();
    let blank = |out: &mut [u8], from: usize, to: usize| {
        for byte in &mut out[from..to] {
            if *byte != b'\n' && *byte != b'\r' {
                *byte = b' ';
            }
        }
    };
    let mut i = 0;
    while i < n {
        match b[i] {
            b'/' if b.get(i + 1) == Some(&b'/') => {
                let end = b[i..].iter().position(|&c| c == b'\n').map_or(n, |p| i + p);
                blank(&mut out, i, end);
                i = end;
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                let mut j = i + 2;
                let mut depth = 1;
                while j < n && depth > 0 {
                    if kotlin && b[j] == b'/' && b.get(j + 1) == Some(&b'*') {
                        depth += 1;
                        j += 2;
                    } else if b[j] == b'*' && b.get(j + 1) == Some(&b'/') {
                        depth -= 1;
                        j += 2;
                    } else {
                        j += 1;
                    }
                }
                let end = j.min(n);
                blank(&mut out, i, end);
                i = end;
            }
            b'"' if b[i..].starts_with(b"\"\"\"") => {
                // Текстовый блок Java / raw-строка Kotlin: до закрывающих `"""`.
                i = text[i + 3..].find("\"\"\"").map_or(n, |p| i + 3 + p + 3);
            }
            b'"' => {
                let mut j = i + 1;
                while j < n && b[j] != b'"' && b[j] != b'\n' {
                    j += if b[j] == b'\\' { 2 } else { 1 };
                }
                i = j + 1;
            }
            b'\'' => {
                // Символьный литерал (`'"'`, `'\''`), чтобы кавычка в нём не открыла строку.
                i += if b.get(i + 1) == Some(&b'\\') {
                    b[i + 2..]
                        .iter()
                        .skip(1)
                        .take(8)
                        .position(|&c| c == b'\'')
                        .map_or(1, |p| p + 4)
                } else if b.get(i + 2) == Some(&b'\'') {
                    3
                } else {
                    1
                };
            }
            _ => i += 1,
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| text.to_string())
}

/// Делит список на элементы по запятым верхнего уровня (строки и скобки учитываются);
/// на несбалансированной закрывающей скобке останавливается (так берётся первый аргумент вызова).
fn split_top(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut depth, mut start) = (0i32, 0usize);
    let (mut in_str, mut esc) = (false, false);
    let mut tail = true;
    for (i, c) in s.char_indices() {
        if in_str {
            if esc {
                esc = false;
            } else if c == '\\' {
                esc = true;
            } else if c == '"' {
                in_str = false;
            }
            continue;
        }
        match c {
            '"' => in_str = true,
            '(' | '{' | '[' => depth += 1,
            ')' | '}' | ']' => {
                if depth == 0 {
                    out.push(&s[start..i]);
                    tail = false;
                    break;
                }
                depth -= 1;
            }
            ',' if depth == 0 => {
                out.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    if tail {
        out.push(&s[start..]);
    }
    out.retain(|e| !e.trim().is_empty());
    out
}

/// Элементы значения аннотации: содержимое `{…}` / `[…]` или единственное выражение.
fn elements_of(expr: &str) -> Vec<&str> {
    let e = expr.trim();
    if (e.starts_with('{') && e.ends_with('}')) || (e.starts_with('[') && e.ends_with(']')) {
        split_top(&e[1..e.len() - 1])
    } else if e.is_empty() {
        Vec::new()
    } else {
        vec![e]
    }
}

/// Содержимое одиночного строкового литерала `"…"` (без конкатенации); иначе `None`.
fn unquote(s: &str) -> Option<&str> {
    let s = s.trim();
    let inner = s.strip_prefix('"')?.strip_suffix('"')?;
    (!inner.contains('"')).then_some(inner)
}

/// Выражение одной строкой для диагностики.
fn flat(expr: &str) -> String {
    expr.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn join(base: &str, path: &str) -> String {
    let b = base.trim_end_matches('/');
    let b = if b.is_empty() || b.starts_with('/') {
        b.to_string()
    } else {
        format!("/{b}")
    };
    let p = path.trim();
    if p.is_empty() {
        return if b.is_empty() { "/".into() } else { b };
    }
    format!("{b}/{}", p.trim_start_matches('/'))
}

/// Первый путь из значения (`"/x"`, `{"/x", "/y"}`, `["/x"]`) и признак «путей несколько».
fn literal_path(expr: &str) -> Result<(String, bool), ()> {
    let els = elements_of(expr);
    let Some(first) = els.first() else {
        return Ok((String::new(), false));
    };
    let lit = unquote(first).ok_or(())?;
    Ok((lit.to_string(), els.len() > 1))
}

/// Путь из аргументов аннотации: строковый литерал (позиционный, `value=`, `path=`, массив
/// `{…}`/`[…]`) и признак «путей несколько»; `Err`, если аргументы есть, а литерала нет (константа).
fn path_of(args: Option<&str>) -> Result<(String, bool), ()> {
    let Some(args) = args else {
        return Ok((String::new(), false));
    };
    let elems = split_top(args);
    let Some(first) = elems.first().map(|e| e.trim()) else {
        return Ok((String::new(), false));
    };
    if first.starts_with(['"', '{', '[']) {
        return literal_path(first);
    }
    for e in &elems {
        if let Some(c) = ELEM_PATH.captures(e.trim()) {
            return literal_path(&c[1]);
        }
    }
    if elems.iter().all(|e| ELEM_OTHER.is_match(e.trim())) {
        return Ok((String::new(), false));
    }
    Err(())
}

/// Сигнатура метода после аннотации: имя и текст параметров до тела.
fn signature(text: &str, from: usize, kotlin: bool) -> Option<(String, &str)> {
    let tail = &text[from..];
    let m = if kotlin {
        KOTLIN_METHOD.captures(tail)?
    } else {
        JAVA_METHOD.captures(tail)?
    };
    let name = m[1].to_string();
    let open = m.get(0)?.end() - 1;
    let mut depth = 0i32;
    for (i, c) in tail[open..].char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some((name, &tail[open..open + i + 1]));
                }
            }
            _ => {}
        }
    }
    None
}

/// Имя query-параметра из атрибутов `@RequestParam(...)`: `"x"`, `value = "x"`, `name = "x"`.
fn param_name(attrs: &str) -> Option<String> {
    let elems = split_top(attrs);
    let first = elems.first()?.trim();
    if let Some(n) = unquote(first) {
        return Some(n.to_string()).filter(|n| !n.is_empty());
    }
    elems.iter().find_map(|e| {
        let c = ELEM_PARAM_NAME.captures(e.trim())?;
        unquote(&c[1]).filter(|n| !n.is_empty()).map(String::from)
    })
}

/// Схема запроса по параметрам метода: path/query/body.
fn request_schema(params: &str, kotlin: bool) -> Option<SchemaSummary> {
    let (pv, qv, body) = if kotlin {
        (&*PATH_VAR_KOTLIN, &*QUERY_KOTLIN, &*BODY_KOTLIN)
    } else {
        (&*PATH_VAR_JAVA, &*QUERY_JAVA, &*BODY_JAVA)
    };
    let mut children = Vec::new();
    for m in pv.captures_iter(params) {
        let n = m.get(1).map(|x| x.as_str()).unwrap_or(&m[2]);
        children.push(SchemaSummary::leaf(format!("path.{n}"), "string").required(true));
    }
    for m in qv.captures_iter(params) {
        let attrs = m.get(1).map(|x| x.as_str()).unwrap_or("");
        let n = param_name(attrs).unwrap_or_else(|| m[2].to_string());
        let required = !attrs.contains("required = false")
            && !attrs.contains("required=false")
            && !attrs.contains("defaultValue");
        children.push(SchemaSummary::leaf(format!("query.{n}"), "string").required(required));
    }
    if let Some(m) = body.captures(params) {
        children.push(SchemaSummary::leaf("body", m[1].trim()).required(true));
    }
    (!children.is_empty()).then(|| SchemaSummary::object("request", children))
}

/// Топик из выражения: литерал без плейсхолдеров `$…` или `None` (тогда нужно предупреждение).
fn topic_literal(expr: &str) -> Option<&str> {
    unquote(expr).filter(|t| !t.is_empty() && !t.contains('$'))
}

pub fn parse(input: &ContractInput, opts: &ParseOptions) -> Result<ParseResult, ParseError> {
    let original = std::str::from_utf8(input.bytes).map_err(|e| ParseError {
        message: format!("not UTF-8: {e}"),
        line: None,
    })?;
    let kotlin = input.path.ends_with(".kt");
    // Дальше везде маска: комментарии не сканируются, строки остаются.
    let masked = mask_comments(original, kotlin);
    let text = masked.as_str();
    let mut out = ParseResult::default();
    let class_at = CLASS
        .captures(text)
        .and_then(|c| c.name("kw"))
        .map_or(0, |m| m.start());
    let base = MAPPING
        .captures_iter(text)
        .take_while(|c| c.get(0).map(|m| m.start() < class_at).unwrap_or(false))
        .find(|c| &c[1] == "Request")
        .map(|c| {
            let line = line_at(text, c.get(0).expect("match").start());
            (path_of(c.get(2).map(|m| m.as_str())), line)
        });
    let base = match base {
        None => String::new(),
        Some((Ok((p, multi)), line)) => {
            if multi {
                out.warn(input.path, Some(line), "several paths, the first is taken");
            }
            p
        }
        Some((Err(()), _)) => {
            out.warn(
                input.path,
                Some(1),
                "class-level @RequestMapping path is not a string literal: paths not computed",
            );
            return Ok(out);
        }
    };
    for cap in MAPPING.captures_iter(text) {
        let whole = cap.get(0).expect("match");
        if whole.start() < class_at {
            continue;
        }
        let line = line_at(text, whole.start());
        let args = cap.get(2).map(|m| m.as_str());
        let mut methods: Vec<String> = Vec::new();
        match &cap[1] {
            "Request" => {
                for m in args.iter().flat_map(|a| METHOD_ATTR.captures_iter(a)) {
                    let v = m[1].to_ascii_uppercase();
                    if !methods.contains(&v) {
                        methods.push(v);
                    }
                }
                if methods.is_empty() {
                    out.warn(
                        input.path,
                        Some(line),
                        "@RequestMapping without method: operation skipped",
                    );
                    continue;
                }
            }
            verb => methods.push(verb.to_ascii_uppercase()),
        }
        let path = match path_of(args) {
            Ok((p, multi)) => {
                if multi {
                    out.warn(input.path, Some(line), "several paths, the first is taken");
                }
                join(&base, &p)
            }
            Err(()) => {
                out.warn(
                    input.path,
                    Some(line),
                    "mapping path is not a string literal (constant?): path not computed",
                );
                continue;
            }
        };
        let Some((name, params)) = signature(text, whole.end(), kotlin) else {
            out.warn(
                input.path,
                Some(line),
                "method signature after the mapping was not recognized",
            );
            continue;
        };
        for method in methods {
            let mut d = OperationDraft::new(
                &opts.service,
                OpKind::Http,
                Address::Http {
                    method,
                    path: path.clone(),
                },
                Source {
                    kind: SourceKind::Spring,
                    path: input.path.into(),
                    line: Some(line),
                },
            );
            d.name = name.clone();
            d.request = request_schema(params, kotlin);
            out.operations.push(d);
        }
    }
    for cap in LISTENER.captures_iter(text) {
        let whole = cap.get(0).expect("match");
        let line = line_at(text, whole.start());
        let topics = split_top(&cap[1])
            .iter()
            .find_map(|e| ELEM_TOPICS.captures(e.trim()).map(|c| c[1].to_string()));
        let elements = topics.as_deref().map(elements_of).unwrap_or_default();
        if elements.is_empty() {
            out.warn(
                input.path,
                Some(line),
                "@KafkaListener without literal topics: operation skipped",
            );
            continue;
        }
        let handler = signature(text, whole.end(), kotlin).map(|s| s.0);
        for el in elements {
            let Some(topic) = topic_literal(el) else {
                out.warn(
                    input.path,
                    Some(line),
                    format!("topic is not a string literal: {}", flat(el)),
                );
                continue;
            };
            let mut d = OperationDraft::new(
                &opts.service,
                OpKind::KafkaConsume,
                Address::Kafka {
                    topic: topic.into(),
                },
                Source {
                    kind: SourceKind::Spring,
                    path: input.path.into(),
                    line: Some(line),
                },
            );
            d.name = handler.clone().unwrap_or_else(|| topic.to_string());
            out.operations.push(d);
        }
    }
    for m in SEND.find_iter(text) {
        let line = line_at(text, m.start());
        let Some(first) = split_top(&text[m.end()..]).first().copied() else {
            continue;
        };
        let Some(topic) = topic_literal(first) else {
            out.warn(
                input.path,
                Some(line),
                format!("topic is not a string literal: {}", flat(first)),
            );
            continue;
        };
        out.operations.push(OperationDraft::new(
            &opts.service,
            OpKind::KafkaProduce,
            Address::Kafka {
                topic: topic.into(),
            },
            Source {
                kind: SourceKind::Spring,
                path: input.path.into(),
                line: Some(line),
            },
        ));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONTROLLER: &str =
        include_str!("../../../dev/fixtures/spring-java/src/main/java/demo/OrderController.java");
    const LISTENER_SRC: &str =
        include_str!("../../../dev/fixtures/spring-java/src/main/java/demo/OrderListener.java");
    const KOTLIN: &str = include_str!(
        "../../../dev/fixtures/spring-kotlin/src/main/kotlin/demo/PaymentController.kt"
    );

    fn run(path: &str, text: &str) -> ParseResult {
        let read = |_: &str| None;
        parse(
            &ContractInput {
                path,
                bytes: text.as_bytes(),
                read: &read,
            },
            &ParseOptions {
                service: "orders".into(),
                ..Default::default()
            },
        )
        .unwrap()
    }

    fn names(s: &SchemaSummary) -> Vec<(&str, &str, bool)> {
        s.children
            .iter()
            .map(|c| (c.name.as_str(), c.kind.as_str(), c.required))
            .collect()
    }

    #[test]
    fn java_controller_with_class_prefix_and_constant_path() {
        let out = run("src/main/java/demo/OrderController.java", CONTROLLER);
        assert_eq!(
            out.operations
                .iter()
                .map(|o| o.id.as_str())
                .collect::<Vec<_>>(),
            [
                "http:orders:POST /v1/orders",
                "http:orders:GET /v1/orders/{id}",
                "http:orders:DELETE /v1/orders/{id}"
            ]
        );
        let create = &out.operations[0];
        assert_eq!(
            (create.name.as_str(), create.source.line),
            ("create", Some(9))
        );
        assert_eq!(
            names(create.request.as_ref().unwrap()),
            vec![("body", "CreateOrderRequest", true)]
        );
        let get = &out.operations[1];
        assert_eq!(
            names(get.request.as_ref().unwrap()),
            vec![
                ("path.id", "string", true),
                ("query.expand", "string", false)
            ]
        );
        assert_eq!(out.operations[2].name, "delete");
        assert_eq!(out.warnings.len(), 1);
        assert_eq!(out.warnings[0].line, Some(23));
        assert!(out.warnings[0].message.contains("not a string literal"));
    }

    #[test]
    fn kafka_listener_and_template_send() {
        let out = run("src/main/java/demo/OrderListener.java", LISTENER_SRC);
        assert_eq!(
            out.operations
                .iter()
                .map(|o| o.id.as_str())
                .collect::<Vec<_>>(),
            [
                "kafka_consume:orders.created",
                "kafka_consume:orders.paid",
                "kafka_produce:orders.audit"
            ]
        );
        assert_eq!(out.operations[0].name, "onOrder");
        assert_eq!(out.operations[2].source.line, Some(11));
    }

    #[test]
    fn kotlin_controller() {
        let out = run("src/main/kotlin/demo/PaymentController.kt", KOTLIN);
        assert_eq!(
            out.operations
                .iter()
                .map(|o| o.id.as_str())
                .collect::<Vec<_>>(),
            [
                "http:orders:POST /v1/payments",
                "http:orders:GET /v1/payments/{id}"
            ]
        );
        assert_eq!(
            names(out.operations[0].request.as_ref().unwrap()),
            vec![("body", "PaymentRequest", true)]
        );
        assert_eq!(
            names(out.operations[1].request.as_ref().unwrap()),
            vec![("path.id", "string", true)]
        );
        assert_eq!(out.operations[1].name, "get");
    }

    #[test]
    fn files_without_mappings_give_nothing() {
        let out = run(
            "src/main/java/demo/Util.java",
            "package demo;\npublic class Util { public static int x() { return 1; } }\n",
        );
        assert!(out.operations.is_empty() && out.warnings.is_empty());
    }

    fn ids(out: &ParseResult) -> Vec<&str> {
        out.operations.iter().map(|o| o.id.as_str()).collect()
    }

    #[test]
    fn comments_are_not_scanned() {
        let src = r#"package demo;

/**
 * Controller class for orders. {@code @GetMapping("/javadoc")}
 */
@RestController
@RequestMapping("/v1/orders")
public class OrderController {
    // @GetMapping("/old")
    /* @PostMapping("/older") */
    @GetMapping("/new")
    public void get() {}
}
"#;
        let out = run("src/main/java/demo/C.java", src);
        assert_eq!(ids(&out), ["http:orders:GET /v1/orders/new"]);
        assert!(out.warnings.is_empty(), "{:?}", out.warnings);
        assert_eq!(out.operations[0].source.line, Some(11));
    }

    #[test]
    fn strings_with_comment_markers_are_kept() {
        let src = "@RestController\n@RequestMapping(\"/v1\")\n@Tag(description = \"class // object\")\npublic class C {\n    @GetMapping(\"/a//b\") // tail\n    public void get() {}\n}\n";
        let out = run("src/main/java/demo/C.java", src);
        assert_eq!(ids(&out), ["http:orders:GET /v1/a//b"]);
    }

    #[test]
    fn mask_keeps_length_and_newlines() {
        let src = "a // x\u{44b}\n/* b\n c */ \"//k\"\n";
        let m = mask_comments(src, false);
        assert_eq!(m.len(), src.len());
        assert_eq!(m.matches('\n').count(), 3);
        assert!(m.contains("\"//k\""));
        assert!(!m.contains('x'));
    }

    #[test]
    fn kotlin_named_array_forms() {
        let src = "@RestController\n@RequestMapping(path = [\"/api\"])\nclass C {\n    @GetMapping(value = [\"/x\"])\n    fun x(): String = TODO()\n}\n";
        let out = run("src/main/kotlin/demo/C.kt", src);
        assert_eq!(ids(&out), ["http:orders:GET /api/x"]);
        assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    }

    #[test]
    fn several_paths_take_the_first_and_warn() {
        let src = "@RestController\npublic class C {\n    @GetMapping({\"/a\", \"/b\"})\n    public void a() {}\n}\n";
        let out = run("src/main/java/demo/C.java", src);
        assert_eq!(ids(&out), ["http:orders:GET /a"]);
        assert_eq!(out.warnings.len(), 1);
        assert_eq!(out.warnings[0].line, Some(3));
        assert!(out.warnings[0].message.contains("several paths"));
    }

    #[test]
    fn request_mapping_with_several_methods() {
        let src = "@RestController\npublic class C {\n    @RequestMapping(path = \"/m\", method = {RequestMethod.GET, RequestMethod.POST})\n    public void m(@RequestParam(\"q\") String q) {}\n}\n";
        let out = run("src/main/java/demo/C.java", src);
        assert_eq!(ids(&out), ["http:orders:GET /m", "http:orders:POST /m"]);
        assert_eq!(
            names(out.operations[1].request.as_ref().unwrap()),
            vec![("query.q", "string", true)]
        );
        let kt = "@RestController\nclass C {\n    @RequestMapping(path = [\"/m\"], method = [RequestMethod.PUT, RequestMethod.PATCH])\n    fun m() {}\n}\n";
        let out = run("src/main/kotlin/demo/C.kt", kt);
        assert_eq!(ids(&out), ["http:orders:PUT /m", "http:orders:PATCH /m"]);
    }

    #[test]
    fn kafka_constants_warn_and_literals_stay() {
        let src = "public class L {\n    @KafkaListener(topics = {Topics.A, \"b\", \"${prop.topic}\"})\n    public void on(String p) {\n        kafkaTemplate.send(TOPIC, p);\n    }\n}\n";
        let out = run("src/main/java/demo/L.java", src);
        assert_eq!(ids(&out), ["kafka_consume:b"]);
        let w: Vec<_> = out
            .warnings
            .iter()
            .map(|w| (w.line, w.message.as_str()))
            .collect();
        assert_eq!(
            w,
            vec![
                (Some(2), "topic is not a string literal: Topics.A"),
                (Some(2), "topic is not a string literal: \"${prop.topic}\""),
                (Some(4), "topic is not a string literal: TOPIC"),
            ]
        );
    }

    #[test]
    fn kafka_listener_single_constant_warns() {
        let src = "class L {\n    @KafkaListener(topics = Topics.A, groupId = \"g\")\n    fun on(p: String) {}\n}\n";
        let out = run("src/main/kotlin/demo/L.kt", src);
        assert!(out.operations.is_empty());
        assert_eq!(out.warnings.len(), 1);
        assert_eq!(out.warnings[0].line, Some(2));
    }

    #[test]
    fn request_param_name_from_literal() {
        let src = "@RestController\npublic class C {\n    @GetMapping(\"/u\")\n    public void u(@RequestParam(\"user_id\") String userId, @RequestParam(name = \"n_x\", required = false) String nx, @RequestParam String plain) {}\n}\n";
        let out = run("src/main/java/demo/C.java", src);
        assert_eq!(
            names(out.operations[0].request.as_ref().unwrap()),
            vec![
                ("query.user_id", "string", true),
                ("query.n_x", "string", false),
                ("query.plain", "string", true)
            ]
        );
    }

    #[test]
    fn kotlin_annotations_on_class_line_keep_base_path() {
        let src = "@RestController @RequestMapping(\"/api\") class Foo {\n    @GetMapping(\"/x\")\n    fun x() = 1\n}\n";
        let out = run("src/main/kotlin/demo/Foo.kt", src);
        assert_eq!(ids(&out), ["http:orders:GET /api/x"]);
        assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    }

    #[test]
    fn java_annotation_before_class_on_same_line_keeps_base_path() {
        let src = "package demo;\n@RequestMapping(\"/v1\")\n@RestController public class X {\n    @GetMapping(\"/y\")\n    public void y() {}\n}\n";
        let out = run("src/main/java/demo/X.java", src);
        assert_eq!(ids(&out), ["http:orders:GET /v1/y"]);
        assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    }

    #[test]
    fn class_regex_ignores_calls_and_class_type() {
        assert!(!CLASS.is_match("val k = classify(x)\n"));
        assert!(!CLASS.is_match("    Class<?> c = foo();\n"));
        assert!(CLASS.is_match("@Foo(a = (1)) fun interface Bar {\n"));
        assert!(CLASS.is_match("    companion object Names {\n"));
    }

    #[test]
    fn kotlin_request_param_literal_name() {
        let src = "@RestController\nclass C {\n    @GetMapping(\"/u\")\n    fun u(@RequestParam(\"user_id\") userId: String) = 1\n}\n";
        let out = run("src/main/kotlin/demo/C.kt", src);
        assert_eq!(
            names(out.operations[0].request.as_ref().unwrap()),
            vec![("query.user_id", "string", true)]
        );
    }

    #[test]
    fn escaped_quote_char_literal_does_not_swallow_next_line() {
        let src = "@RestController\nclass C {\n    val q = '\\''\n    // @GetMapping(\"/no\")\n    @GetMapping(\"/yes\")\n    fun a() = 1\n}\n";
        let masked = mask_comments(src, true);
        assert!(!masked.contains("/no"), "{masked}");
        let out = run("src/main/kotlin/demo/C.kt", src);
        assert_eq!(ids(&out), ["http:orders:GET /yes"]);
    }

    #[test]
    fn join_adds_leading_slash() {
        assert_eq!(join("v1", "x"), "/v1/x");
        assert_eq!(join("v1/", ""), "/v1");
        assert_eq!(join("", "x"), "/x");
    }
}
