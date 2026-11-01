//! Проверки `expect` шага (спека L2 §3.4): статус, JSONPath, схема ответа, задержка.

use glafira_load_model::schema::SchemaSummary;
use glafira_load_model::suite::{Expect, Matcher, SchemaExpect, StatusExpect};
use serde_json::Value;
use serde_json_path::JsonPath;

use crate::errors::{StepCode, StepError};

fn status_matches(expect: &StatusExpect, status: u16) -> bool {
    match expect {
        StatusExpect::One(s) => *s == status,
        StatusExpect::Many(list) => list.contains(&status),
        StatusExpect::Class(class) => class
            .chars()
            .next()
            .and_then(|c| c.to_digit(10))
            .is_some_and(|d| u32::from(status / 100) == d),
    }
}

fn status_label(expect: &StatusExpect) -> String {
    match expect {
        StatusExpect::One(s) => s.to_string(),
        StatusExpect::Many(list) => format!("{list:?}"),
        StatusExpect::Class(c) => c.clone(),
    }
}

fn kind_of(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(n) if n.is_i64() || n.is_u64() => "integer",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn text_of(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Несоответствия значения сжатой схеме: `path: ...`. Типы `string|integer|number|boolean|
/// object|array` проверяются, `required`-поля — на наличие, дети — рекурсивно; узлы с
/// `truncated` (обрезанные уровни) и прочие типы (`any`, `enum`, `one_of`, `bytes`, proto)
/// принимаются как есть. `null` у необязательного поля допустим.
pub fn check_schema(schema: &SchemaSummary, value: &Value) -> Vec<String> {
    let mut out = Vec::new();
    check_node(schema, value, "$", &mut out);
    out
}

fn check_node(schema: &SchemaSummary, value: &Value, path: &str, out: &mut Vec<String>) {
    if value.is_null() && !schema.required {
        return;
    }
    match schema.kind.as_str() {
        "object" => {
            let Value::Object(map) = value else {
                out.push(format!("{path}: expected object, got {}", kind_of(value)));
                return;
            };
            if schema.truncated && schema.children.is_empty() {
                return;
            }
            for child in &schema.children {
                let child_path = format!("{path}.{}", child.name);
                match map.get(&child.name) {
                    Some(v) => check_node(child, v, &child_path, out),
                    None if child.required => {
                        out.push(format!("{child_path}: required field is missing"))
                    }
                    None => {}
                }
            }
        }
        "array" => {
            let Value::Array(items) = value else {
                out.push(format!("{path}: expected array, got {}", kind_of(value)));
                return;
            };
            if let Some(item_schema) = schema.children.first() {
                for (i, item) in items.iter().enumerate() {
                    check_node(item_schema, item, &format!("{path}[{i}]"), out);
                }
            }
        }
        "string" | "boolean" | "integer" => {
            let actual = kind_of(value);
            if actual != schema.kind {
                out.push(format!("{path}: expected {}, got {actual}", schema.kind));
            }
        }
        "number" if !value.is_number() => {
            out.push(format!("{path}: expected number, got {}", kind_of(value)));
        }
        _ => {}
    }
}

fn jsonpath_error(path: &str, message: impl Into<String>) -> StepError {
    StepError::new(
        StepCode::ExpectJsonpath,
        format!("{path}: {}", message.into()),
    )
}

fn check_matcher(path: &str, matcher: &Matcher, body: &Value) -> Result<(), StepError> {
    let query =
        JsonPath::parse(path).map_err(|e| jsonpath_error(path, format!("bad jsonpath: {e}")))?;
    let nodes = query.query(body);
    let first = nodes.first();
    match matcher {
        Matcher::Exists(_) => {
            if first.is_none() {
                return Err(jsonpath_error(path, "no value"));
            }
        }
        Matcher::Equals { equals } => {
            let v = first.ok_or_else(|| jsonpath_error(path, "no value"))?;
            if v != equals {
                return Err(jsonpath_error(path, format!("expected {equals}, got {v}")));
            }
        }
        Matcher::Matches { matches } => {
            let v = first.ok_or_else(|| jsonpath_error(path, "no value"))?;
            let re = regex::Regex::new(matches)
                .map_err(|e| jsonpath_error(path, format!("bad regex `{matches}`: {e}")))?;
            let text = text_of(v);
            if !re.is_match(&text) {
                return Err(jsonpath_error(
                    path,
                    format!("`{text}` does not match /{matches}/"),
                ));
            }
        }
        Matcher::Gt { gt } => {
            let v = first.ok_or_else(|| jsonpath_error(path, "no value"))?;
            let n = v
                .as_f64()
                .ok_or_else(|| jsonpath_error(path, format!("{v} is not a number")))?;
            if n <= *gt {
                return Err(jsonpath_error(path, format!("{n} is not > {gt}")));
            }
        }
        Matcher::Lt { lt } => {
            let v = first.ok_or_else(|| jsonpath_error(path, "no value"))?;
            let n = v
                .as_f64()
                .ok_or_else(|| jsonpath_error(path, format!("{v} is not a number")))?;
            if n >= *lt {
                return Err(jsonpath_error(path, format!("{n} is not < {lt}")));
            }
        }
    }
    Ok(())
}

/// Все проверки шага по порядку: `status`, `jsonpath`, `schema`, `max_ms`. Первая
/// непройденная — ошибка с кодом `EXPECT_*`.
pub fn check_expect(
    expect: &Expect,
    status: Option<u16>,
    body: Option<&Value>,
    latency_ms: f64,
    response_schema: Option<&SchemaSummary>,
) -> Result<(), StepError> {
    if let Some(want) = &expect.status {
        match status {
            Some(s) if status_matches(want, s) => {}
            Some(s) => {
                return Err(StepError::new(
                    StepCode::ExpectStatus,
                    format!("expected status {}, got {s}", status_label(want)),
                ));
            }
            None => {
                return Err(StepError::new(
                    StepCode::ExpectStatus,
                    format!("expected status {}, got no status", status_label(want)),
                ));
            }
        }
    }
    for (path, matcher) in &expect.jsonpath {
        let body = body.ok_or_else(|| jsonpath_error(path, "no JSON body"))?;
        check_matcher(path, matcher, body)?;
    }
    if let Some(SchemaExpect::Response) = expect.schema {
        let schema = response_schema.ok_or_else(|| {
            StepError::new(
                StepCode::ExpectSchema,
                "the operation has no response schema in the registry",
            )
        })?;
        let body = body.ok_or_else(|| StepError::new(StepCode::ExpectSchema, "no JSON body"))?;
        let problems = check_schema(schema, body);
        if !problems.is_empty() {
            return Err(StepError::new(StepCode::ExpectSchema, problems.join("; ")));
        }
    }
    if let Some(max) = expect.max_ms
        && latency_ms > max as f64
    {
        return Err(StepError::new(
            StepCode::ExpectMaxMs,
            format!("{latency_ms:.1} ms is over {max} ms"),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use glafira_load_model::suite::ExistsTag;
    use serde_json::json;
    use std::collections::BTreeMap;

    fn with_status(s: StatusExpect) -> Expect {
        Expect {
            status: Some(s),
            ..Default::default()
        }
    }

    fn with_path(path: &str, m: Matcher) -> Expect {
        Expect {
            jsonpath: BTreeMap::from([(path.to_string(), m)]),
            ..Default::default()
        }
    }

    fn code(r: Result<(), StepError>) -> Option<StepCode> {
        r.err().map(|e| e.code)
    }

    #[test]
    fn status_one_many_and_class() {
        assert!(
            check_expect(
                &with_status(StatusExpect::One(201)),
                Some(201),
                None,
                0.0,
                None
            )
            .is_ok()
        );
        assert_eq!(
            code(check_expect(
                &with_status(StatusExpect::One(201)),
                Some(500),
                None,
                0.0,
                None
            )),
            Some(StepCode::ExpectStatus)
        );
        let many = with_status(StatusExpect::Many(vec![200, 204]));
        assert!(check_expect(&many, Some(204), None, 0.0, None).is_ok());
        assert_eq!(
            code(check_expect(&many, Some(201), None, 0.0, None)),
            Some(StepCode::ExpectStatus)
        );
        let class = with_status(StatusExpect::Class("2xx".into()));
        assert!(check_expect(&class, Some(299), None, 0.0, None).is_ok());
        assert_eq!(
            code(check_expect(&class, Some(301), None, 0.0, None)),
            Some(StepCode::ExpectStatus)
        );
        let bad = with_status(StatusExpect::Class("xx".into()));
        assert_eq!(
            code(check_expect(&bad, Some(200), None, 0.0, None)),
            Some(StepCode::ExpectStatus)
        );
        let e = check_expect(&class, None, None, 0.0, None).unwrap_err();
        assert_eq!(e.code, StepCode::ExpectStatus);
        assert!(e.message.contains("no status"), "{}", e.message);
        assert!(
            check_expect(&Expect::default(), Some(500), None, 0.0, None).is_ok(),
            "без expect статус не проверяется"
        );
    }

    #[test]
    fn jsonpath_exists_equals_matches_gt_lt() {
        let body = json!({"id": "A-1", "qty": 3, "price": 2.5, "tags": ["x"], "name": 7});
        let ok = |e: &Expect| check_expect(e, Some(200), Some(&body), 1.0, None).is_ok();
        let fails = |e: &Expect| code(check_expect(e, Some(200), Some(&body), 1.0, None));

        assert!(ok(&with_path("$.id", Matcher::Exists(ExistsTag::Exists))));
        assert_eq!(
            fails(&with_path("$.missing", Matcher::Exists(ExistsTag::Exists))),
            Some(StepCode::ExpectJsonpath)
        );

        assert!(ok(&with_path(
            "$.id",
            Matcher::Equals {
                equals: json!("A-1")
            }
        )));
        assert!(ok(&with_path(
            "$.qty",
            Matcher::Equals { equals: json!(3) }
        )));
        assert_eq!(
            fails(&with_path("$.qty", Matcher::Equals { equals: json!("3") })),
            Some(StepCode::ExpectJsonpath)
        );
        assert_eq!(
            fails(&with_path(
                "$.missing",
                Matcher::Equals { equals: json!(1) }
            )),
            Some(StepCode::ExpectJsonpath)
        );

        assert!(ok(&with_path(
            "$.id",
            Matcher::Matches {
                matches: "^A-\\d+$".into()
            }
        )));
        assert!(
            ok(&with_path(
                "$.name",
                Matcher::Matches {
                    matches: "^7$".into()
                }
            )),
            "число сравнивается как текст"
        );
        assert_eq!(
            fails(&with_path(
                "$.id",
                Matcher::Matches {
                    matches: "^B".into()
                }
            )),
            Some(StepCode::ExpectJsonpath)
        );
        let e = check_expect(
            &with_path(
                "$.id",
                Matcher::Matches {
                    matches: "(".into(),
                },
            ),
            Some(200),
            Some(&body),
            1.0,
            None,
        )
        .unwrap_err();
        assert!(e.message.contains("bad regex"), "{}", e.message);

        assert!(ok(&with_path("$.qty", Matcher::Gt { gt: 2.0 })));
        assert!(
            ok(&with_path("$.price", Matcher::Gt { gt: 2.0 })),
            "число сравнивается как f64"
        );
        assert_eq!(
            fails(&with_path("$.qty", Matcher::Gt { gt: 3.0 })),
            Some(StepCode::ExpectJsonpath)
        );
        assert_eq!(
            fails(&with_path("$.id", Matcher::Gt { gt: 0.0 })),
            Some(StepCode::ExpectJsonpath)
        );
        assert!(ok(&with_path("$.qty", Matcher::Lt { lt: 3.5 })));
        assert_eq!(
            fails(&with_path("$.qty", Matcher::Lt { lt: 3.0 })),
            Some(StepCode::ExpectJsonpath)
        );

        assert!(ok(&with_path(
            "$.tags[0]",
            Matcher::Equals { equals: json!("x") }
        )));
        let e = check_expect(
            &with_path("$[", Matcher::Exists(ExistsTag::Exists)),
            Some(200),
            Some(&body),
            1.0,
            None,
        )
        .unwrap_err();
        assert!(e.message.contains("bad jsonpath"), "{}", e.message);
        let e = check_expect(
            &with_path("$.id", Matcher::Exists(ExistsTag::Exists)),
            Some(200),
            None,
            1.0,
            None,
        )
        .unwrap_err();
        assert_eq!(
            (e.code, e.message.as_str()),
            (StepCode::ExpectJsonpath, "$.id: no JSON body")
        );
    }

    fn order_schema() -> SchemaSummary {
        SchemaSummary::object(
            "body",
            vec![
                SchemaSummary::leaf("id", "string").required(true),
                SchemaSummary::leaf("qty", "integer"),
                SchemaSummary::leaf("total", "number"),
                SchemaSummary::leaf("paid", "boolean"),
                SchemaSummary::array(
                    "items",
                    SchemaSummary::object(
                        "x",
                        vec![SchemaSummary::leaf("sku", "string").required(true)],
                    ),
                ),
                SchemaSummary::leaf("meta", "any"),
                SchemaSummary {
                    truncated: true,
                    ..SchemaSummary::object("deep", vec![])
                },
            ],
        )
    }

    #[test]
    fn schema_response_checks_types_required_and_nesting() {
        let schema = order_schema();
        let expect = Expect {
            schema: Some(SchemaExpect::Response),
            ..Default::default()
        };
        let good = json!({"id": "A", "qty": 2, "total": 2.5, "paid": true, "items": [{"sku": "S"}], "meta": [1], "deep": {"anything": 1}, "extra": 1});
        assert!(check_expect(&expect, Some(200), Some(&good), 1.0, Some(&schema)).is_ok());
        let also_good = json!({"id": "A", "total": 3, "qty": null});
        assert!(
            check_expect(&expect, Some(200), Some(&also_good), 1.0, Some(&schema)).is_ok(),
            "integer подходит под number, null у необязательного поля допустим"
        );

        let problems = check_schema(
            &schema,
            &json!({"qty": "2", "total": "x", "paid": 1, "items": [{"id": 1}], "deep": 5}),
        );
        assert_eq!(
            problems,
            vec![
                "$.id: required field is missing",
                "$.qty: expected integer, got string",
                "$.total: expected number, got string",
                "$.paid: expected boolean, got integer",
                "$.items[0].sku: required field is missing",
                "$.deep: expected object, got integer",
            ]
        );
        assert_eq!(
            check_schema(&schema, &json!({"id": "A", "items": {}})),
            vec!["$.items: expected array, got object"]
        );
        assert_eq!(
            check_schema(&schema, &json!([])),
            vec!["$: expected object, got array"]
        );
        assert_eq!(
            check_schema(&schema, &json!({"id": null})),
            vec!["$.id: expected string, got null"]
        );

        let e = check_expect(&expect, Some(200), Some(&json!({})), 1.0, Some(&schema)).unwrap_err();
        assert_eq!(e.code, StepCode::ExpectSchema);
        assert!(
            e.message.contains("$.id: required field is missing"),
            "{}",
            e.message
        );
        let e = check_expect(&expect, Some(200), Some(&good), 1.0, None).unwrap_err();
        assert_eq!(e.code, StepCode::ExpectSchema);
        assert!(e.message.contains("no response schema"), "{}", e.message);
        let e = check_expect(&expect, Some(200), None, 1.0, Some(&schema)).unwrap_err();
        assert_eq!(
            (e.code, e.message.as_str()),
            (StepCode::ExpectSchema, "no JSON body")
        );
    }

    #[test]
    fn max_ms_and_check_order() {
        let expect = Expect {
            max_ms: Some(800),
            ..Default::default()
        };
        assert!(check_expect(&expect, Some(200), None, 800.0, None).is_ok());
        let e = check_expect(&expect, Some(200), None, 800.5, None).unwrap_err();
        assert_eq!(e.code, StepCode::ExpectMaxMs);
        assert_eq!(e.message, "800.5 ms is over 800 ms");
        // Порядок: статус раньше jsonpath, jsonpath раньше max_ms.
        let all = Expect {
            status: Some(StatusExpect::One(200)),
            jsonpath: BTreeMap::from([("$.id".to_string(), Matcher::Exists(ExistsTag::Exists))]),
            schema: None,
            max_ms: Some(1),
        };
        assert_eq!(
            code(check_expect(&all, Some(500), Some(&json!({})), 5.0, None)),
            Some(StepCode::ExpectStatus)
        );
        assert_eq!(
            code(check_expect(&all, Some(200), Some(&json!({})), 5.0, None)),
            Some(StepCode::ExpectJsonpath)
        );
        assert_eq!(
            code(check_expect(
                &all,
                Some(200),
                Some(&json!({"id": 1})),
                5.0,
                None
            )),
            Some(StepCode::ExpectMaxMs)
        );
        assert!(check_expect(&all, Some(200), Some(&json!({"id": 1})), 1.0, None).is_ok());
    }
}
