//! Файлы схем без AsyncAPI: Avro `.avsc` и JSON Schema (спека L1 §5.2, решение плана 6).
//! Топик — из `kafka.topics` манифеста, иначе из имени файла (`topic_guessed`).

use std::collections::BTreeMap;

use glafira_load_model::project::Direction;
use glafira_load_model::registry::{Address, OpKind, OperationDraft, Source, SourceKind};
use glafira_load_model::schema::{MAX_DEPTH, SchemaSummary};
use serde_json::Value;

use crate::summary::{Resolver, summarize};
use crate::{ContractInput, ParseError, ParseOptions, ParseResult, parse_document};

fn avro_primitive(t: &str) -> Option<&'static str> {
    Some(match t {
        "null" => "null",
        "boolean" => "boolean",
        "int" | "long" => "integer",
        "float" | "double" => "number",
        "bytes" | "fixed" => "bytes",
        "string" => "string",
        _ => return None,
    })
}

/// Полное имя именованного типа Avro: `namespace.name`, если namespace задан (у вложенных — унаследован).
fn full_name(schema: &Value, namespace: &str) -> String {
    let name = schema.get("name").and_then(Value::as_str).unwrap_or("");
    let ns = schema
        .get("namespace")
        .and_then(Value::as_str)
        .unwrap_or(namespace);
    if name.contains('.') || ns.is_empty() {
        name.into()
    } else {
        format!("{ns}.{name}")
    }
}

fn avro(
    name: &str,
    schema: &Value,
    required: bool,
    namespace: &str,
    named: &mut BTreeMap<String, Value>,
    depth: usize,
    stack: &mut Vec<String>,
) -> SchemaSummary {
    match schema {
        Value::String(t) => {
            if let Some(k) = avro_primitive(t) {
                return SchemaSummary::leaf(name, k).required(required);
            }
            let fq = if t.contains('.') || namespace.is_empty() {
                t.clone()
            } else {
                format!("{namespace}.{t}")
            };
            let Some(def) = named.get(&fq).or_else(|| named.get(t)).cloned() else {
                return SchemaSummary::leaf(name, format!("unresolved:{t}")).required(required);
            };
            if stack.contains(&fq) {
                return SchemaSummary::leaf(
                    name,
                    format!("ref:{}", fq.rsplit('.').next().unwrap_or(&fq)),
                )
                .required(required);
            }
            stack.push(fq);
            let out = avro(name, &def, required, namespace, named, depth, stack);
            stack.pop();
            out
        }
        Value::Array(union) => {
            let members: Vec<&Value> = union
                .iter()
                .filter(|m| m.as_str() != Some("null"))
                .collect();
            let nullable = members.len() != union.len();
            match members.as_slice() {
                [one] => avro(
                    name,
                    one,
                    required && !nullable,
                    namespace,
                    named,
                    depth,
                    stack,
                ),
                _ => {
                    let mut s = SchemaSummary::leaf(name, "one_of").required(required && !nullable);
                    s.children = members
                        .iter()
                        .enumerate()
                        .map(|(i, m)| {
                            avro(
                                &format!("variant{}", i + 1),
                                m,
                                false,
                                namespace,
                                named,
                                depth + 1,
                                stack,
                            )
                        })
                        .collect();
                    s
                }
            }
        }
        Value::Object(_) => {
            let t = schema.get("type").cloned().unwrap_or(Value::Null);
            let mut out = SchemaSummary::leaf(name, "any").required(required);
            if depth >= MAX_DEPTH {
                out.truncated = true;
                return out;
            }
            match t.as_str() {
                Some("record") => {
                    let fq = full_name(schema, namespace);
                    named.insert(fq.clone(), schema.clone());
                    let ns = schema
                        .get("namespace")
                        .and_then(Value::as_str)
                        .unwrap_or(namespace);
                    out.kind = "object".into();
                    stack.push(fq);
                    if let Some(fields) = schema.get("fields").and_then(Value::as_array) {
                        for f in fields {
                            let fname = f.get("name").and_then(Value::as_str).unwrap_or("?");
                            let ftype = f.get("type").cloned().unwrap_or(Value::Null);
                            let req = f.get("default").is_none();
                            let mut child = avro(fname, &ftype, req, ns, named, depth + 1, stack);
                            if let Some(d) = f.get("default").filter(|d| !d.is_null()) {
                                child.example = Some(d.to_string());
                            }
                            out.children.push(child);
                        }
                    }
                    stack.pop();
                }
                Some("enum") => {
                    named.insert(full_name(schema, namespace), schema.clone());
                    out.kind = "enum".into();
                    out.enum_values = schema
                        .get("symbols")
                        .and_then(Value::as_array)
                        .map(|s| {
                            s.iter()
                                .filter_map(Value::as_str)
                                .map(String::from)
                                .collect()
                        })
                        .unwrap_or_default();
                }
                Some("array") => {
                    let items = schema.get("items").cloned().unwrap_or(Value::Null);
                    out = SchemaSummary::array(
                        name,
                        avro("items", &items, false, namespace, named, depth + 1, stack),
                    )
                    .required(required);
                }
                Some("map") => {
                    let values = schema.get("values").cloned().unwrap_or(Value::Null);
                    out.kind = "map".into();
                    out.children = vec![avro(
                        "value",
                        &values,
                        false,
                        namespace,
                        named,
                        depth + 1,
                        stack,
                    )];
                }
                Some("fixed") => {
                    named.insert(full_name(schema, namespace), schema.clone());
                    out.kind = "bytes".into();
                }
                Some(_) => return avro(name, &t, required, namespace, named, depth, stack),
                None => {}
            }
            out
        }
        _ => SchemaSummary::leaf(name, "any").required(required),
    }
}

fn file_stem(path: &str) -> &str {
    let base = path.rsplit('/').next().unwrap_or(path);
    base.split_once('.').map(|(s, _)| s).unwrap_or(base)
}

pub fn parse(input: &ContractInput, opts: &ParseOptions) -> Result<ParseResult, ParseError> {
    let doc = parse_document(input.bytes)?;
    let mut out = ParseResult::default();
    let is_avro =
        input.path.ends_with(".avsc") || doc.get("type").and_then(Value::as_str) == Some("record");
    let mut summary = if is_avro {
        avro(
            "message",
            &doc,
            false,
            "",
            &mut BTreeMap::new(),
            1,
            &mut vec![],
        )
    } else {
        let r = Resolver::new(input.path, &doc, input.read);
        let s = summarize("message", &doc, false, input.path, &r, 1, &mut vec![]);
        out.warnings.extend(r.take_warnings());
        s
    };
    summary.limit();
    let (topic, direction, guessed) = match opts.topics.get(input.path) {
        Some(b) => (b.topic.clone(), b.direction, false),
        None => (file_stem(input.path).to_string(), Direction::Produce, true),
    };
    if guessed {
        out.warn(input.path, None, format!("topic {topic:?} is guessed from the file name: add the file to kafka.topics of the service"));
    }
    let kind = match direction {
        Direction::Produce => OpKind::KafkaProduce,
        Direction::Consume => OpKind::KafkaConsume,
    };
    let mut d = OperationDraft::new(
        &opts.service,
        kind,
        Address::Kafka {
            topic: topic.clone(),
        },
        Source {
            kind: SourceKind::Schema,
            path: input.path.into(),
            line: Some(1),
        },
    );
    d.name = topic;
    d.topic_guessed = guessed;
    match kind {
        OpKind::KafkaProduce => d.request = Some(summary),
        _ => d.response = Some(summary),
    }
    out.operations.push(d);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TopicBinding;

    const AVSC: &str = include_str!("../../../dev/fixtures/schemas/orders-created.avsc");
    const JSON_SCHEMA: &str = include_str!("../../../dev/fixtures/schemas/payment.schema.json");

    fn child<'a>(s: &'a SchemaSummary, name: &str) -> &'a SchemaSummary {
        s.children.iter().find(|c| c.name == name).unwrap()
    }

    #[test]
    fn avro_without_binding_guesses_the_topic() {
        let read = |_: &str| None;
        let out = parse(
            &ContractInput {
                path: "schemas/orders-created.avsc",
                bytes: AVSC.as_bytes(),
                read: &read,
            },
            &ParseOptions {
                service: "orders".into(),
                ..Default::default()
            },
        )
        .unwrap();
        let op = &out.operations[0];
        assert_eq!(
            (op.id.as_str(), op.topic_guessed),
            ("kafka_produce:orders-created", true)
        );
        assert!(out.warnings[0].message.contains("guessed"));
        let m = op.request.as_ref().unwrap();
        assert!(child(m, "orderId").required);
        let total = child(m, "total");
        assert_eq!((total.kind.as_str(), total.required), ("number", false));
        assert_eq!(child(m, "status").enum_values, ["NEW", "PAID"]);
        assert_eq!(child(&child(m, "items").children[0], "qty").kind, "integer");
        assert_eq!(child(m, "labels").children[0].kind, "string");
        assert_eq!(child(m, "parent").kind, "ref:OrderCreated");
        assert_eq!(child(m, "amount").kind, "bytes");
    }

    #[test]
    fn binding_sets_topic_and_direction() {
        let read = |_: &str| None;
        let mut opts = ParseOptions {
            service: "orders".into(),
            ..Default::default()
        };
        opts.topics.insert(
            "schemas/orders-created.avsc".into(),
            TopicBinding {
                topic: "orders.created".into(),
                direction: Direction::Consume,
            },
        );
        let out = parse(
            &ContractInput {
                path: "schemas/orders-created.avsc",
                bytes: AVSC.as_bytes(),
                read: &read,
            },
            &opts,
        )
        .unwrap();
        let op = &out.operations[0];
        assert_eq!(
            (op.id.as_str(), op.topic_guessed),
            ("kafka_consume:orders.created", false)
        );
        assert!(op.request.is_none() && op.response.is_some());
        assert!(out.warnings.is_empty());
    }

    #[test]
    fn json_schema_files_use_the_generic_summarizer() {
        let read = |_: &str| None;
        let out = parse(
            &ContractInput {
                path: "schemas/payment.schema.json",
                bytes: JSON_SCHEMA.as_bytes(),
                read: &read,
            },
            &ParseOptions {
                service: "pay".into(),
                ..Default::default()
            },
        )
        .unwrap();
        let op = &out.operations[0];
        assert_eq!(op.id, "kafka_produce:payment");
        let m = op.request.as_ref().unwrap();
        assert!(child(m, "paymentId").required && !child(m, "amount").required);
    }
}
