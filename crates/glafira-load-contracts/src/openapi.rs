//! OpenAPI 3.0/3.1 и Swagger 2.0 → операции `http` (спека L1 §5.2).

use std::collections::BTreeMap;

use glafira_load_model::registry::{Address, OpKind, OperationDraft, Source, SourceKind};
use glafira_load_model::schema::SchemaSummary;
use serde_json::Value;

use crate::summary::{Resolver, summarize};
use crate::{ContractInput, ParseError, ParseOptions, ParseResult, line_of, parse_document};

const METHODS: [&str; 8] = [
    "get", "put", "post", "delete", "options", "head", "patch", "trace",
];

#[derive(Clone, Copy, PartialEq)]
enum Version {
    V2,
    V3,
}

/// Узел или то, на что он ссылается (параметры, тела и ответы бывают `$ref`).
fn deref(r: &Resolver, file: &str, node: &Value) -> (String, Value) {
    match node.get("$ref").and_then(Value::as_str) {
        Some(reference) => r
            .resolve(file, reference)
            .unwrap_or_else(|| (file.to_string(), Value::Null)),
        None => (file.to_string(), node.clone()),
    }
}

fn first_media_schema(content: Option<&Value>) -> Option<&Value> {
    content?.as_object()?.values().next()?.get("schema")
}

fn str_of<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

pub fn parse(input: &ContractInput, opts: &ParseOptions) -> Result<ParseResult, ParseError> {
    let doc = parse_document(input.bytes)?;
    let text = String::from_utf8_lossy(input.bytes);
    let version = if doc.get("openapi").is_some() {
        Version::V3
    } else if doc.get("swagger").is_some() {
        Version::V2
    } else {
        return Err(ParseError {
            message: "neither an OpenAPI nor a Swagger document (no openapi/swagger field)".into(),
            line: None,
        });
    };
    let r = Resolver::new(input.path, &doc, input.read);
    let file = input.path;
    let mut out = ParseResult::default();
    let Some(paths) = doc.get("paths").and_then(Value::as_object) else {
        out.warn(file, None, "no paths");
        return Ok(out);
    };
    for (path, item) in paths {
        let Some(item) = item.as_object() else {
            continue;
        };
        let line =
            line_of(&text, &format!("{path}:")).or_else(|| line_of(&text, &format!("\"{path}\"")));
        let shared: Vec<Value> = item
            .get("parameters")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for method in METHODS {
            let Some(op) = item.get(method) else {
                continue;
            };
            let mut draft = OperationDraft::new(
                &opts.service,
                OpKind::Http,
                Address::Http {
                    method: method.to_ascii_uppercase(),
                    path: path.clone(),
                },
                Source {
                    kind: SourceKind::Openapi,
                    path: file.into(),
                    line,
                },
            );
            if let Some(id) = str_of(op, "operationId") {
                draft.name = id.into();
            }
            draft.summary = str_of(op, "summary")
                .or_else(|| str_of(op, "description"))
                .unwrap_or("")
                .into();
            if op.get("callbacks").is_some() || op.get("links").is_some() {
                out.warn(
                    file,
                    line,
                    format!("{}: callbacks and links are not supported", draft.name),
                );
            }
            // Запрос: параметры пути и операции, затем тело.
            let mut children = Vec::new();
            let own: Vec<Value> = op
                .get("parameters")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let mut stack = Vec::new();
            for p in shared.iter().chain(own.iter()) {
                let (pfile, p) = deref(&r, file, p);
                let (Some(name), Some(loc)) = (str_of(&p, "name"), str_of(&p, "in")) else {
                    continue;
                };
                let required =
                    loc == "path" || p.get("required").and_then(Value::as_bool).unwrap_or(false);
                match (version, loc) {
                    (Version::V2, "body") => {
                        if let Some(schema) = p.get("schema") {
                            children.push(summarize(
                                "body", schema, required, &pfile, &r, 2, &mut stack,
                            ));
                        }
                    }
                    (Version::V2, _) => children.push(
                        SchemaSummary::leaf(
                            format!("{loc}.{name}"),
                            str_of(&p, "type").unwrap_or("string"),
                        )
                        .required(required),
                    ),
                    (Version::V3, _) => {
                        let kind = p
                            .get("schema")
                            .map(|s| summarize(name, s, required, &pfile, &r, 2, &mut stack).kind)
                            .unwrap_or_else(|| "string".into());
                        children.push(
                            SchemaSummary::leaf(format!("{loc}.{name}"), kind).required(required),
                        );
                    }
                }
            }
            if version == Version::V3
                && let Some(body) = op.get("requestBody")
            {
                let (bfile, body) = deref(&r, file, body);
                let required = body
                    .get("required")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                if let Some(schema) = first_media_schema(body.get("content")) {
                    children.push(summarize(
                        "body", schema, required, &bfile, &r, 2, &mut stack,
                    ));
                }
            }
            if !children.is_empty() {
                let mut req = SchemaSummary::object("request", children);
                req.limit();
                draft.request = Some(req);
            }
            // Ответы: первый 2xx — `response`, остальные — `responses`.
            let mut responses: BTreeMap<String, SchemaSummary> = BTreeMap::new();
            if let Some(resps) = op.get("responses").and_then(Value::as_object) {
                for (code, resp) in resps {
                    let (rfile, resp) = deref(&r, file, resp);
                    let schema = match version {
                        Version::V3 => first_media_schema(resp.get("content")),
                        Version::V2 => resp.get("schema"),
                    };
                    let mut s = match schema {
                        Some(s) => summarize("response", s, false, &rfile, &r, 1, &mut stack),
                        None => SchemaSummary::leaf("response", "empty"),
                    };
                    s.limit();
                    responses.insert(code.clone(), s);
                }
            }
            if let Some(code) = responses.keys().find(|c| c.starts_with('2')).cloned() {
                draft.response = responses.remove(&code);
            }
            draft.responses = responses;
            out.operations.push(draft);
        }
    }
    out.warnings.extend(r.take_warnings());
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const OPENAPI: &str = include_str!("../../../dev/fixtures/orders-openapi/openapi.yaml");
    const ORDER: &str = include_str!("../../../dev/fixtures/orders-openapi/schemas/order.yaml");
    const SWAGGER: &str = include_str!("../../../dev/fixtures/orders-swagger/swagger.json");

    fn opts() -> ParseOptions {
        ParseOptions {
            service: "orders".into(),
            ..Default::default()
        }
    }

    fn names(s: &SchemaSummary) -> Vec<&str> {
        s.children.iter().map(|c| c.name.as_str()).collect()
    }

    #[test]
    fn openapi_3_operations_with_external_refs() {
        let read = |p: &str| (p == "api/schemas/order.yaml").then(|| ORDER.as_bytes().to_vec());
        let input = ContractInput {
            path: "api/openapi.yaml",
            bytes: OPENAPI.as_bytes(),
            read: &read,
        };
        let out = parse(&input, &opts()).unwrap();
        assert_eq!(
            out.operations
                .iter()
                .map(|o| o.id.as_str())
                .collect::<Vec<_>>(),
            [
                "http:orders:POST /v1/orders",
                "http:orders:GET /v1/orders/{id}"
            ]
        );
        let post = &out.operations[0];
        assert_eq!(
            (post.name.as_str(), post.summary.as_str()),
            ("createOrder", "Create an order")
        );
        assert_eq!(post.source.line, Some(4));
        let req = post.request.as_ref().unwrap();
        assert_eq!(names(req), ["body"]);
        let body = &req.children[0];
        assert!(body.required);
        assert_eq!(names(body), ["items", "note"]);
        assert_eq!(
            body.children[0].children[0]
                .children
                .iter()
                .map(|c| c.name.as_str())
                .collect::<Vec<_>>(),
            ["sku", "qty"],
            "Item из соседнего файла по локальной ссылке"
        );
        assert_eq!(body.children[1].kind, "string", "[string, null] → string");
        let resp = post.response.as_ref().unwrap();
        assert_eq!(names(resp), ["id", "status", "items", "customer"]);
        assert_eq!(resp.children[1].enum_values, ["NEW", "PAID", "FAILED"]);
        assert_eq!(resp.children[3].children[1].kind, "ref:Customer");
        assert_eq!(post.responses.keys().collect::<Vec<_>>(), ["400"]);
        let get = &out.operations[1];
        let req = get.request.as_ref().unwrap();
        assert_eq!(names(req), ["path.id", "query.expand"]);
        assert!(req.children[0].required && !req.children[1].required);
        assert_eq!(get.responses["404"].kind, "empty");
        assert_eq!(out.warnings.len(), 1);
        assert!(out.warnings[0].message.contains("callbacks"));
    }

    #[test]
    fn swagger_2_body_and_header_params() {
        let read = |_: &str| None;
        let input = ContractInput {
            path: "swagger.json",
            bytes: SWAGGER.as_bytes(),
            read: &read,
        };
        let out = parse(&input, &opts()).unwrap();
        let op = &out.operations[0];
        assert_eq!(op.id, "http:orders:POST /legacy/pay");
        let req = op.request.as_ref().unwrap();
        assert_eq!(names(req), ["header.X-Request-Id", "body"]);
        assert_eq!(names(&req.children[1]), ["amount", "currency"]);
        assert!(req.children[1].children[0].required);
        assert_eq!(names(op.response.as_ref().unwrap()), ["id"]);
    }

    #[test]
    fn not_openapi_is_an_error_and_missing_ref_file_warns() {
        let read = |_: &str| None;
        assert!(
            parse(
                &ContractInput {
                    path: "x.yaml",
                    bytes: b"info: {}\n",
                    read: &read
                },
                &opts()
            )
            .is_err()
        );
        let out = parse(
            &ContractInput {
                path: "api/openapi.yaml",
                bytes: OPENAPI.as_bytes(),
                read: &read,
            },
            &opts(),
        )
        .unwrap();
        let body = &out.operations[0].request.as_ref().unwrap().children[0];
        assert_eq!(body.kind, "unresolved");
        assert!(
            out.warnings
                .iter()
                .any(|w| w.message.contains("$ref not found")),
            "{:?}",
            out.warnings
        );
    }
}
