//! Заготовка сьюта из операций реестра (спека §7.1): сценарий на операцию, тело по сжатой
//! схеме запроса с генераторами по типам, `expect.status` по коду 2xx из `responses`.

use std::collections::{BTreeMap, BTreeSet};

use glafira_load_model::project::is_id;
use glafira_load_model::registry::{Address, OpKind, Operation, Registry, address_label};
use glafira_load_model::schema::SchemaSummary;
pub use glafira_load_model::suite::DEFAULT_CONSUME_TIMEOUT_MS;
use glafira_load_model::suite::{
    Action, ConsumeStep, Defaults, Encoding, Expect, FORMAT, HttpStep, Match, OnFail, ProduceStep,
    Scenario, StatusExpect, Step, Suite, to_yaml,
};
use serde_json::Value;

use crate::error::{AppError, AppResult, ErrorCode};
use crate::project::manifest::ProjectDir;
use crate::suites::files::{bad_name, suite_file};

fn is_uuid_name(name: &str) -> bool {
    let l = name.to_ascii_lowercase();
    l == "id" || l == "uuid" || l.ends_with("_id") || l.ends_with("uuid") || name.ends_with("Id")
}

fn is_time_name(name: &str) -> bool {
    let l = name.to_ascii_lowercase();
    l.ends_with("_at") || name.ends_with("At") || l.contains("date") || l.contains("time")
}

/// Значение-генератор по сжатой схеме: `integer` → `{{int 1 100}}`, `number` → `{{float 0 1}}`,
/// `boolean` → `true`, `array` → один элемент, `object` → рекурсивно, перечисление → первое
/// значение, строки — по имени поля (`{{uuid}}`, `{{now iso}}`, иначе `{{str 8}}`).
pub fn value_for(s: &SchemaSummary) -> Value {
    if let Some(first) = s.enum_values.first() {
        return Value::String(first.clone());
    }
    match s.kind.as_str() {
        "object" => Value::Object(
            s.children
                .iter()
                .map(|c| (c.name.clone(), value_for(c)))
                .collect(),
        ),
        "array" => Value::Array(vec![
            s.children
                .first()
                .map(value_for)
                .unwrap_or_else(|| Value::String("{{str 8}}".into())),
        ]),
        "one_of" => s.children.first().map(value_for).unwrap_or(Value::Null),
        "integer" => Value::String("{{int 1 100}}".into()),
        "number" => Value::String("{{float 0 1}}".into()),
        "boolean" => Value::Bool(true),
        _ if is_uuid_name(&s.name) => Value::String("{{uuid}}".into()),
        _ if is_time_name(&s.name) => Value::String("{{now iso}}".into()),
        _ => Value::String("{{str 8}}".into()),
    }
}

/// Скалярное значение строкой (параметры запроса, заголовки, `body_text`).
fn scalar_text(s: &SchemaSummary) -> String {
    match value_for(s) {
        Value::String(t) => t,
        other => other.to_string(),
    }
}

/// `[a-z0-9-]{1,64}` из произвольного текста: прочее → `-`, повторы схлопываются; пусто → `step`.
pub fn slug(text: &str) -> String {
    let mut out = String::new();
    for ch in text.chars() {
        let c = ch.to_ascii_lowercase();
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    let out: String = out.chars().take(64).collect();
    let out = out.trim_end_matches('-');
    if out.is_empty() {
        "step".into()
    } else {
        out.to_string()
    }
}

/// Имена параметров `{x}` в пути операции.
fn path_params(path: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = path;
    while let Some(start) = rest.find('{') {
        let after = &rest[start + 1..];
        let Some(end) = after.find('}') else {
            break;
        };
        let name = after[..end].trim();
        if !name.is_empty() {
            out.push(name.to_string());
        }
        rest = &after[end + 1..];
    }
    out
}

/// Подпись сценария: адрес операции, у Kafka — с направлением.
fn label(op: &Operation) -> String {
    let address = address_label(&op.draft.address);
    match op.draft.kind {
        OpKind::KafkaProduce => format!("produce {address}"),
        OpKind::KafkaConsume => format!("consume {address}"),
        _ => address,
    }
}

fn unsupported(id: &str, kind: &str) -> AppError {
    app_err!(
        ErrorCode::InvalidInput,
        "errors.suite.unsupportedKind",
        "Operation {{id}} of kind {{kind}} cannot be used in a suite",
        id = id,
        kind = kind
    )
}

fn not_found(id: &str) -> AppError {
    app_err!(
        ErrorCode::NotFound,
        "errors.registry.notFound",
        "Operation {{id}} not found",
        id = id
    )
}

/// Тело HTTP-шага по схеме `body`: объект, массив и вариант — JSON по схеме; скаляр —
/// `body_text`; `empty` — без тела; тип, которого сжатая схема не раскрывает (класс Spring,
/// `ref:…`, `any`), — пустой объект для правки руками.
fn body_of(s: &SchemaSummary) -> (Option<Value>, Option<String>) {
    match s.kind.as_str() {
        "object" | "array" | "one_of" => (Some(value_for(s)), None),
        "string" | "integer" | "number" | "boolean" | "bytes" | "enum" => {
            (None, Some(scalar_text(s)))
        }
        "empty" => (None, None),
        _ => (Some(Value::Object(Default::default())), None),
    }
}

/// `expect.status`: код первого 2xx известен, только если `response` пуст, а 2xx лежит в
/// `responses` (импортёр кладёт первый 2xx в `response` без кода) — иначе класс `2xx`.
fn status_of(op: &Operation) -> StatusExpect {
    let listed = op
        .draft
        .responses
        .keys()
        .find(|k| k.starts_with('2'))
        .and_then(|k| k.parse::<u16>().ok());
    match listed {
        Some(code) if op.draft.response.is_none() => StatusExpect::One(code),
        _ => StatusExpect::Class("2xx".into()),
    }
}

/// HTTP-шаг: параметры пути, обязательные параметры запроса и заголовки, тело по схеме.
fn http_step(op: &Operation, addr: &str) -> HttpStep {
    let mut step = HttpStep {
        operation: op.draft.id.clone(),
        path: BTreeMap::new(),
        query: BTreeMap::new(),
        headers: BTreeMap::new(),
        body: None,
        body_text: None,
        timeout_ms: None,
    };
    for name in path_params(addr) {
        step.path.insert(name, "{{str 6}}".to_string());
    }
    let Some(req) = &op.draft.request else {
        return step;
    };
    // Импортёры пишут `request` с полями `path.*`, `query.*`, `header.*`, `body`; схема,
    // записанная руками, бывает и самим телом (`name: body`).
    if req.name == "body" {
        (step.body, step.body_text) = body_of(req);
        return step;
    }
    for c in &req.children {
        if let Some(name) = c.name.strip_prefix("path.") {
            step.path.insert(name.to_string(), "{{str 6}}".to_string());
        } else if let Some(name) = c.name.strip_prefix("query.") {
            if c.required {
                step.query.insert(name.to_string(), scalar_text(c));
            }
        } else if let Some(name) = c.name.strip_prefix("header.") {
            if c.required {
                step.headers.insert(name.to_string(), scalar_text(c));
            }
        } else if c.name == "body" {
            (step.body, step.body_text) = body_of(c);
        }
    }
    step
}

/// Шаг для операции: HTTP — с `expect.status`; Kafka — значение по схеме сообщения или пустое
/// ожидание. gRPC и SQL в сьют не попадают (`INVALID_INPUT`).
fn step_for(op: &Operation, id: String) -> AppResult<Step> {
    let operation = op.draft.id.clone();
    let (action, expect) = match (op.draft.kind, &op.draft.address) {
        (OpKind::Http, Address::Http { path, .. }) => (
            Action::Http(http_step(op, path)),
            Expect {
                status: Some(status_of(op)),
                ..Default::default()
            },
        ),
        // Схема сообщения продюсера — в `request` (импортёры `asyncapi` и `schema`).
        (OpKind::KafkaProduce, _) => (
            Action::KafkaProduce(ProduceStep {
                operation,
                key: Some("{{uuid}}".into()),
                value: Some(
                    op.draft
                        .request
                        .as_ref()
                        .or(op.draft.response.as_ref())
                        .map(value_for)
                        .unwrap_or_else(|| Value::Object(Default::default())),
                ),
                value_text: None,
                headers: BTreeMap::new(),
                encoding: Encoding::Json,
            }),
            Expect::default(),
        ),
        (OpKind::KafkaConsume, _) => (
            Action::KafkaConsume(ConsumeStep {
                operation,
                match_: Match::default(),
                timeout_ms: DEFAULT_CONSUME_TIMEOUT_MS,
            }),
            Expect::default(),
        ),
        (kind, _) => return Err(unsupported(&op.draft.id, kind.as_str())),
    };
    Ok(Step {
        id: Some(id),
        action,
        expect,
        extract: BTreeMap::new(),
    })
}

/// Сьют: сценарий на операцию в порядке `ids`; `id` сценария и шага — из адреса операции,
/// повторы получают суффикс `-2`, `-3`, ….
pub fn build(name: &str, registry: &Registry, ids: &[String]) -> AppResult<Suite> {
    let mut scenarios = Vec::new();
    let mut used = BTreeSet::new();
    for id in ids {
        let op = registry.get(id).ok_or_else(|| not_found(id))?;
        let base = slug(&label(op));
        let mut sid = base.clone();
        let mut n = 1;
        while !used.insert(sid.clone()) {
            n += 1;
            sid = format!("{base}-{n}");
        }
        let step = step_for(op, sid.clone())?;
        scenarios.push(Scenario {
            id: sid,
            drives: None,
            on_fail: OnFail::default(),
            steps: vec![step],
        });
    }
    Ok(Suite {
        format: FORMAT,
        name: name.into(),
        secrets: Vec::new(),
        data: BTreeMap::new(),
        defaults: Defaults::default(),
        scenarios,
    })
}

/// Пишет `suites/<имя>/suite.yaml`; существующий сьют, недопустимое имя или пустой список
/// операций — `INVALID_INPUT`. Возвращает путь файла и сьют.
pub fn create(d: &ProjectDir, name: &str, ids: &[String]) -> AppResult<(String, Suite)> {
    if !is_id(name) {
        return Err(bad_name(name));
    }
    if ids.is_empty() {
        return Err(app_err!(
            ErrorCode::InvalidInput,
            "errors.suite.noOperations",
            "Select at least one operation"
        ));
    }
    let file = suite_file(name);
    if d.file(&file).exists() {
        return Err(app_err!(
            ErrorCode::InvalidInput,
            "errors.suite.exists",
            "Suite {{name}} already exists",
            name = name
        ));
    }
    let suite = build(name, &d.registry()?, ids)?;
    d.write_text(&file, &to_yaml(&suite))?;
    Ok((file, suite))
}

#[cfg(test)]
mod tests {
    use super::*;
    use glafira_load_model::registry::{Manual, OperationDraft, Seen, Source, Status};
    use glafira_load_model::suite::parse;
    use serde_json::json;

    fn op(kind: OpKind, address: Address) -> Operation {
        let draft = OperationDraft::new("orders", kind, address, Source::manual());
        Operation {
            fingerprint: draft.fingerprint(),
            draft,
            status: Status::Active,
            first_seen: Seen::default(),
            last_seen: Seen::default(),
            manual: Manual::default(),
        }
    }

    fn http(method: &str, path: &str) -> Operation {
        op(
            OpKind::Http,
            Address::Http {
                method: method.into(),
                path: path.into(),
            },
        )
    }

    fn registry() -> Registry {
        let mut post = http("POST", "/v1/orders");
        post.draft.request = Some(SchemaSummary::object(
            "request",
            vec![
                SchemaSummary::leaf("query.dry_run", "boolean"),
                SchemaSummary::leaf("header.X-Trace", "string").required(true),
                SchemaSummary::object(
                    "body",
                    vec![
                        SchemaSummary::leaf("customerId", "string").required(true),
                        SchemaSummary::leaf("createdAt", "string"),
                        SchemaSummary::leaf("qty", "integer"),
                        SchemaSummary::leaf("price", "number"),
                        SchemaSummary::leaf("paid", "boolean"),
                        SchemaSummary {
                            name: "status".into(),
                            kind: "enum".into(),
                            enum_values: vec!["NEW".into(), "PAID".into()],
                            ..Default::default()
                        },
                        SchemaSummary::array("tags", SchemaSummary::leaf("x", "string")),
                        SchemaSummary::object(
                            "address",
                            vec![SchemaSummary::leaf("city", "string")],
                        ),
                    ],
                ),
            ],
        ));
        post.draft
            .responses
            .insert("400".into(), SchemaSummary::leaf("response", "empty"));
        let mut get = http("GET", "/v1/orders/{id}");
        get.draft.request = Some(SchemaSummary::object(
            "request",
            vec![
                SchemaSummary::leaf("path.id", "string").required(true),
                SchemaSummary::leaf("query.expand", "string"),
            ],
        ));
        get.draft
            .responses
            .insert("204".into(), SchemaSummary::leaf("response", "empty"));
        get.draft
            .responses
            .insert("404".into(), SchemaSummary::leaf("response", "empty"));
        // Импортёры кладут схему сообщения продюсера в `request` (`asyncapi`, `schema`).
        let mut produce = op(
            OpKind::KafkaProduce,
            Address::Kafka {
                topic: "orders.paid".into(),
            },
        );
        produce.draft.request = Some(SchemaSummary::object(
            "message",
            vec![
                SchemaSummary::leaf("orderId", "string"),
                SchemaSummary::leaf("paidAt", "string"),
            ],
        ));
        let consume = op(
            OpKind::KafkaConsume,
            Address::Kafka {
                topic: "orders.created".into(),
            },
        );
        let sql = op(
            OpKind::Sql,
            Address::Sql {
                name: "orders.select".into(),
            },
        );
        let mut r = Registry::default();
        r.operations.extend([post, get, produce, consume, sql]);
        r.sort();
        r
    }

    #[test]
    fn generators_by_type_and_name() {
        let v = |name: &str, kind: &str| value_for(&SchemaSummary::leaf(name, kind));
        assert_eq!(v("qty", "integer"), json!("{{int 1 100}}"));
        assert_eq!(v("price", "number"), json!("{{float 0 1}}"));
        assert_eq!(v("paid", "boolean"), json!(true));
        assert_eq!(v("name", "string"), json!("{{str 8}}"));
        assert_eq!(v("customerId", "string"), json!("{{uuid}}"));
        assert_eq!(v("order_id", "string"), json!("{{uuid}}"));
        assert_eq!(v("createdAt", "string"), json!("{{now iso}}"));
        assert_eq!(v("due_date", "string"), json!("{{now iso}}"));
        assert_eq!(
            value_for(&SchemaSummary::array(
                "tags",
                SchemaSummary::leaf("x", "integer")
            )),
            json!(["{{int 1 100}}"])
        );
        assert_eq!(slug("POST /v1/orders"), "post-v1-orders");
        assert_eq!(slug("consume orders.created"), "consume-orders-created");
        assert_eq!(slug("///"), "step");
        assert_eq!(slug(&"a".repeat(100)).len(), 64);
    }

    #[test]
    fn builds_a_scenario_per_operation() {
        let r = registry();
        let ids = vec![
            "http:orders:POST /v1/orders".to_string(),
            "http:orders:GET /v1/orders/{id}".into(),
            "kafka_produce:orders.paid".into(),
            "kafka_consume:orders.created".into(),
        ];
        let suite = build("generated", &r, &ids).unwrap();
        assert_eq!((suite.format, suite.name.as_str()), (FORMAT, "generated"));
        assert_eq!(suite.scenarios.len(), 4);
        let ids_of: Vec<&str> = suite.scenarios.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(
            ids_of,
            [
                "post-v1-orders",
                "get-v1-orders-id",
                "produce-orders-paid",
                "consume-orders-created"
            ]
        );
        let post = &suite.scenarios[0].steps[0];
        let Action::Http(h) = &post.action else {
            panic!("http step");
        };
        assert_eq!(h.operation, "http:orders:POST /v1/orders");
        assert!(h.query.is_empty(), "необязательный параметр не попадает");
        assert_eq!(h.headers["X-Trace"], "{{str 8}}");
        assert_eq!(
            h.body,
            Some(json!({
                "customerId": "{{uuid}}",
                "createdAt": "{{now iso}}",
                "qty": "{{int 1 100}}",
                "price": "{{float 0 1}}",
                "paid": true,
                "status": "NEW",
                "tags": ["{{str 8}}"],
                "address": { "city": "{{str 8}}" }
            }))
        );
        assert_eq!(post.expect.status, Some(StatusExpect::Class("2xx".into())));
        let get = &suite.scenarios[1].steps[0];
        let Action::Http(h) = &get.action else {
            panic!("http step");
        };
        assert_eq!(h.path["id"], "{{str 6}}");
        assert_eq!(get.expect.status, Some(StatusExpect::One(204)));
        let Action::KafkaProduce(p) = &suite.scenarios[2].steps[0].action else {
            panic!("produce step");
        };
        assert_eq!(p.key.as_deref(), Some("{{uuid}}"));
        assert_eq!(
            p.value,
            Some(json!({ "orderId": "{{uuid}}", "paidAt": "{{now iso}}" }))
        );
        assert_eq!(p.encoding, Encoding::Json);
        let Action::KafkaConsume(c) = &suite.scenarios[3].steps[0].action else {
            panic!("consume step");
        };
        assert_eq!(c.timeout_ms, DEFAULT_CONSUME_TIMEOUT_MS);
        assert_eq!(c.match_, Match::default());
        let text = to_yaml(&suite);
        assert_eq!(parse(&text).unwrap(), suite, "{text}");
        assert!(text.contains("http:orders:POST /v1/orders"), "{text}");
    }

    #[test]
    fn status_and_body_forms() {
        let mut r = registry();
        // Первый 2xx лежит в `response` без кода: другой 2xx из `responses` не угадывается.
        let get = r.get_mut("http:orders:GET /v1/orders/{id}").unwrap();
        get.draft.response = Some(SchemaSummary::leaf("response", "empty"));
        // Тело — класс из Spring (`body` с именем типа) и тело прямо в `request` (без обёртки).
        let mut put = http("PUT", "/v1/orders/{id}");
        put.draft.request = Some(SchemaSummary::object(
            "request",
            vec![
                SchemaSummary::leaf("path.id", "string").required(true),
                SchemaSummary::leaf("body", "UpdateOrder").required(true),
            ],
        ));
        let mut patch = http("PATCH", "/v1/orders");
        patch.draft.request = Some(SchemaSummary::object(
            "body",
            vec![SchemaSummary::leaf("note", "string")],
        ));
        let mut text = http("POST", "/v1/notes");
        text.draft.request = Some(SchemaSummary::object(
            "request",
            vec![SchemaSummary::leaf("body", "string")],
        ));
        r.operations.extend([put, patch, text]);
        r.sort();
        let ids = [
            "http:orders:GET /v1/orders/{id}",
            "http:orders:PUT /v1/orders/{id}",
            "http:orders:PATCH /v1/orders",
            "http:orders:POST /v1/notes",
        ]
        .map(String::from);
        let suite = build("x", &r, &ids).unwrap();
        let http_of = |i: usize| match &suite.scenarios[i].steps[0].action {
            Action::Http(h) => h.clone(),
            other => panic!("{other:?}"),
        };
        assert_eq!(
            suite.scenarios[0].steps[0].expect.status,
            Some(StatusExpect::Class("2xx".into()))
        );
        assert_eq!(http_of(1).body, Some(json!({})));
        assert_eq!(http_of(1).path["id"], "{{str 6}}");
        assert_eq!(http_of(2).body, Some(json!({ "note": "{{str 8}}" })));
        assert_eq!(
            (http_of(3).body, http_of(3).body_text.as_deref()),
            (None, Some("{{str 8}}"))
        );
    }

    #[test]
    fn duplicate_ids_unknown_and_unsupported_operations() {
        let r = registry();
        let suite = build(
            "x",
            &r,
            &[
                "http:orders:POST /v1/orders".to_string(),
                "http:orders:POST /v1/orders".into(),
            ],
        )
        .unwrap();
        assert_eq!(suite.scenarios[1].id, "post-v1-orders-2");
        assert_eq!(
            build("x", &r, &["nope".to_string()]).unwrap_err().code,
            ErrorCode::NotFound
        );
        let e = build("x", &r, &["sql:orders:orders.select".to_string()]).unwrap_err();
        assert_eq!(
            (e.code, e.key.as_str()),
            (ErrorCode::InvalidInput, "errors.suite.unsupportedKind")
        );
    }

    #[test]
    fn create_writes_the_file_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p").to_string_lossy().into_owned();
        crate::project::manifest::create(&path, "t").unwrap();
        let d = ProjectDir::open(&path).unwrap();
        d.save_registry(&registry()).unwrap();
        let ids = vec!["http:orders:GET /v1/orders/{id}".to_string()];
        let (file, suite) = create(&d, "gen", &ids).unwrap();
        assert_eq!(file, "suites/gen/suite.yaml");
        assert_eq!(d.read_text(&file).unwrap().unwrap(), to_yaml(&suite));
        assert_eq!(
            create(&d, "gen", &ids).unwrap_err().key,
            "errors.suite.exists"
        );
        assert_eq!(
            create(&d, "Gen", &ids).unwrap_err().code,
            ErrorCode::InvalidInput
        );
        assert_eq!(
            create(&d, "empty", &[]).unwrap_err().key,
            "errors.suite.noOperations"
        );
        assert!(!d.file("suites/empty").exists());
    }
}
