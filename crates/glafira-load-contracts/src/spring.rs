//! Spring MVC/WebFlux и Spring Kafka по текстовым правилам, без компиляции (спека L1 §5.2):
//! единственная эвристика по коду в L1. Путь через константу — предупреждение, операция не создаётся.

use std::sync::LazyLock;

use glafira_load_model::registry::{Address, OpKind, OperationDraft, Source, SourceKind};
use glafira_load_model::schema::SchemaSummary;
use regex::Regex;

use crate::{ContractInput, ParseError, ParseOptions, ParseResult};

static CLASS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b(?:class|interface|object)\s+\w+").unwrap());
static MAPPING: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"@(Get|Post|Put|Delete|Patch|Request)Mapping\s*(?:\(((?:[^()]|\([^()]*\))*)\))?")
        .unwrap()
});
static STRING: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#""([^"]*)""#).unwrap());
static NAMED_PATH: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"\b(?:value|path)\s*=\s*(?:\{\s*)?"([^"]*)""#).unwrap());
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
static TOPICS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"\btopics\s*=\s*(\{[^}]*\}|\[[^\]]*\]|"[^"]*")"#).unwrap());
static SEND: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)\w*(?:kafka|template)\w*\s*\.\s*send\s*\(\s*"([^"]+)""#).unwrap()
});

fn line_at(text: &str, pos: usize) -> u32 {
    text[..pos].matches('\n').count() as u32 + 1
}

fn join(base: &str, path: &str) -> String {
    let b = base.trim_end_matches('/');
    let p = path.trim();
    if p.is_empty() {
        return if b.is_empty() { "/".into() } else { b.into() };
    }
    format!("{b}/{}", p.trim_start_matches('/'))
}

/// Путь из аргументов аннотации: строковый литерал (позиционный, `value=`, `path=`) или `None`,
/// если аргументы есть, а литерала нет (константа).
fn path_of(args: Option<&str>) -> Result<String, ()> {
    let Some(args) = args else {
        return Ok(String::new());
    };
    if let Some(m) = NAMED_PATH.captures(args) {
        return Ok(m[1].to_string());
    }
    let trimmed = args.trim();
    if let Some(m) = STRING.captures(trimmed)
        && (trimmed.starts_with('"') || trimmed.starts_with("{\"") || trimmed.starts_with("[\""))
    {
        return Ok(m[1].to_string());
    }
    if trimmed.is_empty()
        || trimmed.starts_with("method")
        || trimmed.starts_with("consumes")
        || trimmed.starts_with("produces")
    {
        return Ok(String::new());
    }
    Err(())
}

/// Сигнатура метода после аннотации: имя и текст параметров до тела.
fn signature(text: &str, from: usize, kotlin: bool) -> Option<(String, &str, usize)> {
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
                    return Some((name, &tail[open..open + i + 1], from + open));
                }
            }
            _ => {}
        }
    }
    None
}

pub fn parse(input: &ContractInput, opts: &ParseOptions) -> Result<ParseResult, ParseError> {
    let text = std::str::from_utf8(input.bytes).map_err(|e| ParseError {
        message: format!("not UTF-8: {e}"),
        line: None,
    })?;
    let kotlin = input.path.ends_with(".kt");
    let mut out = ParseResult::default();
    let class_at = CLASS.find(text).map(|m| m.start()).unwrap_or(0);
    let base = MAPPING
        .captures_iter(text)
        .take_while(|c| c.get(0).map(|m| m.start() < class_at).unwrap_or(false))
        .find(|c| &c[1] == "Request")
        .map(|c| path_of(c.get(2).map(|m| m.as_str())))
        .transpose();
    let base = match base {
        Ok(b) => b.unwrap_or_default(),
        Err(()) => {
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
        let method = match &cap[1] {
            "Request" => match args.and_then(|a| METHOD_ATTR.captures(a)) {
                Some(m) => m[1].to_ascii_uppercase(),
                None => {
                    out.warn(
                        input.path,
                        Some(line),
                        "@RequestMapping without method: operation skipped",
                    );
                    continue;
                }
            },
            verb => verb.to_ascii_uppercase(),
        };
        let path = match path_of(args) {
            Ok(p) => join(&base, &p),
            Err(()) => {
                out.warn(
                    input.path,
                    Some(line),
                    "mapping path is not a string literal (constant?): path not computed",
                );
                continue;
            }
        };
        let Some((name, params, _)) = signature(text, whole.end(), kotlin) else {
            out.warn(
                input.path,
                Some(line),
                "method signature after the mapping was not recognized",
            );
            continue;
        };
        let mut d = OperationDraft::new(
            &opts.service,
            OpKind::Http,
            Address::Http { method, path },
            Source {
                kind: SourceKind::Spring,
                path: input.path.into(),
                line: Some(line),
            },
        );
        d.name = name;
        let mut children = Vec::new();
        let (pv, qv, body) = if kotlin {
            (&*PATH_VAR_KOTLIN, &*QUERY_KOTLIN, &*BODY_KOTLIN)
        } else {
            (&*PATH_VAR_JAVA, &*QUERY_JAVA, &*BODY_JAVA)
        };
        for m in pv.captures_iter(params) {
            let n = m.get(1).map(|x| x.as_str()).unwrap_or(&m[2]);
            children.push(SchemaSummary::leaf(format!("path.{n}"), "string").required(true));
        }
        for m in qv.captures_iter(params) {
            let attrs = m.get(1).map(|x| x.as_str()).unwrap_or("");
            let n = NAMED_PATH
                .captures(attrs)
                .map(|c| c[1].to_string())
                .unwrap_or_else(|| m[2].to_string());
            let required = !attrs.contains("required = false")
                && !attrs.contains("required=false")
                && !attrs.contains("defaultValue");
            children.push(SchemaSummary::leaf(format!("query.{n}"), "string").required(required));
        }
        if let Some(m) = body.captures(params) {
            children.push(SchemaSummary::leaf("body", m[1].trim()).required(true));
        }
        if !children.is_empty() {
            d.request = Some(SchemaSummary::object("request", children));
        }
        out.operations.push(d);
    }
    for cap in LISTENER.captures_iter(text) {
        let line = line_at(text, cap.get(0).expect("match").start());
        let Some(t) = TOPICS.captures(&cap[1]) else {
            out.warn(
                input.path,
                Some(line),
                "@KafkaListener without literal topics: operation skipped",
            );
            continue;
        };
        for topic in STRING.captures_iter(&t[1]) {
            let mut d = OperationDraft::new(
                &opts.service,
                OpKind::KafkaConsume,
                Address::Kafka {
                    topic: topic[1].into(),
                },
                Source {
                    kind: SourceKind::Spring,
                    path: input.path.into(),
                    line: Some(line),
                },
            );
            d.name = signature(text, cap.get(0).expect("match").end(), kotlin)
                .map(|s| s.0)
                .unwrap_or_else(|| topic[1].to_string());
            out.operations.push(d);
        }
    }
    for cap in SEND.captures_iter(text) {
        let line = line_at(text, cap.get(0).expect("match").start());
        let d = OperationDraft::new(
            &opts.service,
            OpKind::KafkaProduce,
            Address::Kafka {
                topic: cap[1].into(),
            },
            Source {
                kind: SourceKind::Spring,
                path: input.path.into(),
                line: Some(line),
            },
        );
        out.operations.push(d);
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
}
