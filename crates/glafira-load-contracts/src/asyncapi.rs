//! AsyncAPI 2.x и 3.x → операции Kafka (спека L1 §5.2): `publish`/`send` → `kafka_produce`,
//! `subscribe`/`receive` → `kafka_consume`. Сообщение produce лежит в `request`, consume — в `response`.

use glafira_load_model::registry::{Address, OpKind, OperationDraft, Source, SourceKind};
use glafira_load_model::schema::SchemaSummary;
use serde_json::Value;

use crate::summary::{Resolver, summarize};
use crate::{ContractInput, ParseError, ParseOptions, ParseResult, line_of, parse_document};

/// Снимает цепочку `$ref` (в v3 сообщение канала может ссылаться на компонент).
fn deref(r: &Resolver, file: &str, node: &Value) -> (String, Value) {
    let (mut file, mut node) = (file.to_string(), node.clone());
    for _ in 0..5 {
        let Some(reference) = node.get("$ref").and_then(Value::as_str).map(String::from) else {
            break;
        };
        match r.resolve(&file, &reference) {
            Some((f, n)) => {
                file = f;
                node = n;
            }
            None => return (file, Value::Null),
        }
    }
    (file, node)
}

/// Глубина вложенных `oneOf` сообщения: самоссылка (`A.oneOf: [{$ref: A}]`) не уводит в
/// бесконечную рекурсию.
const MAX_ONE_OF_DEPTH: usize = 8;

fn message_summary(
    r: &Resolver,
    file: &str,
    message: &Value,
    out: &mut ParseResult,
    path: &str,
    depth: usize,
) -> Option<SchemaSummary> {
    let (mfile, message) = deref(r, file, message);
    if let Some(variants) = message.get("oneOf").and_then(Value::as_array) {
        if depth >= MAX_ONE_OF_DEPTH {
            out.warn(
                path,
                None,
                "message oneOf is nested too deeply (a cycle?): the message is skipped",
            );
            return None;
        }
        if depth == 0 {
            out.warn(path, None, "message oneOf: the first variant is taken");
        }
        return message_summary(r, &mfile, variants.first()?, out, path, depth + 1);
    }
    let payload = message.get("payload")?;
    let mut s = summarize("message", payload, false, &mfile, r, 1, &mut vec![]);
    s.limit();
    Some(s)
}

fn source(path: &str, line: Option<u32>) -> Source {
    Source {
        kind: SourceKind::Asyncapi,
        path: path.into(),
        line,
    }
}

fn push(
    out: &mut ParseResult,
    opts: &ParseOptions,
    kind: OpKind,
    topic: &str,
    (name, summary): (&str, &str),
    source: Source,
    message: Option<SchemaSummary>,
) {
    let mut d = OperationDraft::new(
        &opts.service,
        kind,
        Address::Kafka {
            topic: topic.into(),
        },
        source,
    );
    d.name = name.into();
    d.summary = summary.into();
    match kind {
        OpKind::KafkaProduce => d.request = message,
        _ => d.response = message,
    }
    out.operations.push(d);
}

fn str_of<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}

pub fn parse(input: &ContractInput, opts: &ParseOptions) -> Result<ParseResult, ParseError> {
    let doc = parse_document(input.bytes)?;
    let text = String::from_utf8_lossy(input.bytes);
    let Some(version) = doc.get("asyncapi").and_then(Value::as_str) else {
        return Err(ParseError {
            message: "not an AsyncAPI document (no asyncapi field)".into(),
            line: None,
        });
    };
    let major: u32 = version
        .split('.')
        .next()
        .and_then(|m| m.parse().ok())
        .unwrap_or(0);
    let r = Resolver::new(input.path, &doc, input.read);
    let path = input.path;
    let mut out = ParseResult::default();
    if let Some(servers) = doc.get("servers").and_then(Value::as_object) {
        for (name, s) in servers {
            let (_, s) = deref(&r, path, s);
            let protocol = str_of(&s, "protocol");
            if protocol != "kafka" && protocol != "kafka-secure" {
                out.warn(path, line_of(&text, &format!("{name}:")), format!("server {name}: protocol {protocol} is not Kafka, operations are still read as Kafka topics"));
            }
        }
    }
    match major {
        2 => {
            let Some(channels) = doc.get("channels").and_then(Value::as_object) else {
                return Ok(out);
            };
            for (channel, item) in channels {
                let (cfile, item) = deref(&r, path, item);
                let line = line_of(&text, &format!("{channel}:"));
                for (key, kind) in [
                    ("publish", OpKind::KafkaProduce),
                    ("subscribe", OpKind::KafkaConsume),
                ] {
                    let Some(op) = item.get(key) else { continue };
                    let name = op
                        .get("operationId")
                        .and_then(Value::as_str)
                        .map(String::from)
                        .unwrap_or_else(|| format!("{key} {channel}"));
                    let message = op
                        .get("message")
                        .and_then(|m| message_summary(&r, &cfile, m, &mut out, path, 0));
                    push(
                        &mut out,
                        opts,
                        kind,
                        channel,
                        (&name, str_of(op, "summary")),
                        source(path, line),
                        message,
                    );
                }
            }
        }
        3 => {
            let Some(operations) = doc.get("operations").and_then(Value::as_object) else {
                return Ok(out);
            };
            for (id, op) in operations {
                let (ofile, op) = deref(&r, path, op);
                let kind = match str_of(&op, "action") {
                    "send" => OpKind::KafkaProduce,
                    "receive" => OpKind::KafkaConsume,
                    other => {
                        out.warn(
                            path,
                            None,
                            format!("operation {id}: unknown action {other:?}"),
                        );
                        continue;
                    }
                };
                let Some(channel_ref) = op.get("channel") else {
                    out.warn(path, None, format!("operation {id}: no channel"));
                    continue;
                };
                let (cfile, channel) = deref(&r, &ofile, channel_ref);
                let key = channel_ref
                    .get("$ref")
                    .and_then(Value::as_str)
                    .and_then(|s| s.rsplit('/').next())
                    .unwrap_or("");
                let address = channel
                    .get("address")
                    .and_then(Value::as_str)
                    .unwrap_or(key);
                let message = op
                    .get("messages")
                    .and_then(Value::as_array)
                    .and_then(|m| m.first())
                    .map(|m| (ofile.clone(), m.clone()))
                    .or_else(|| {
                        channel
                            .get("messages")
                            .and_then(Value::as_object)
                            .and_then(|m| m.values().next())
                            .map(|m| (cfile.clone(), m.clone()))
                    })
                    .and_then(|(f, m)| message_summary(&r, &f, &m, &mut out, path, 0));
                push(
                    &mut out,
                    opts,
                    kind,
                    address,
                    (id, str_of(&op, "summary")),
                    source(path, line_of(&text, &format!("{id}:"))),
                    message,
                );
            }
        }
        _ => {
            return Err(ParseError {
                message: format!("unsupported AsyncAPI version {version}"),
                line: None,
            });
        }
    }
    out.warnings.extend(r.take_warnings());
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const V2: &str = include_str!("../../../dev/fixtures/orders-asyncapi/asyncapi-v2.yaml");
    const V3: &str = include_str!("../../../dev/fixtures/orders-asyncapi/asyncapi-v3.yaml");

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
    fn self_referencing_one_of_message_does_not_overflow_the_stack() {
        let doc = "asyncapi: 2.6.0\ninfo: { title: x, version: '1' }\nchannels:\n  t:\n    publish:\n      message: { $ref: '#/components/messages/A' }\ncomponents:\n  messages:\n    A:\n      oneOf:\n        - $ref: '#/components/messages/A'\n";
        let read = |_: &str| None;
        let out = parse(
            &ContractInput {
                path: "asyncapi.yaml",
                bytes: doc.as_bytes(),
                read: &read,
            },
            &opts(),
        )
        .unwrap();
        assert_eq!(out.operations.len(), 1);
        assert!(out.operations[0].request.is_none());
        assert!(
            out.warnings
                .iter()
                .any(|w| w.message.contains("too deeply")),
            "{:?}",
            out.warnings
        );
    }

    #[test]
    fn v2_publish_is_produce_and_subscribe_is_consume() {
        let read = |_: &str| None;
        let out = parse(
            &ContractInput {
                path: "asyncapi.yaml",
                bytes: V2.as_bytes(),
                read: &read,
            },
            &opts(),
        )
        .unwrap();
        assert_eq!(
            out.operations
                .iter()
                .map(|o| o.id.as_str())
                .collect::<Vec<_>>(),
            [
                "kafka_produce:orders.created",
                "kafka_consume:payments.completed"
            ]
        );
        let p = &out.operations[0];
        assert_eq!(
            (p.name.as_str(), p.summary.as_str(), p.source.line),
            ("publishOrderCreated", "Order created", Some(7))
        );
        assert_eq!(names(p.request.as_ref().unwrap()), ["orderId", "total"]);
        assert!(p.response.is_none());
        let c = &out.operations[1];
        assert_eq!(names(c.response.as_ref().unwrap()), ["paymentId"]);
        let msgs: Vec<&str> = out.warnings.iter().map(|w| w.message.as_str()).collect();
        assert!(msgs.iter().any(|m| m.contains("amqp")), "{msgs:?}");
        assert!(msgs.iter().any(|m| m.contains("oneOf")), "{msgs:?}");
    }

    #[test]
    fn v3_send_receive_with_channel_addresses() {
        let read = |_: &str| None;
        let out = parse(
            &ContractInput {
                path: "asyncapi.yaml",
                bytes: V3.as_bytes(),
                read: &read,
            },
            &opts(),
        )
        .unwrap();
        assert_eq!(
            out.operations
                .iter()
                .map(|o| o.id.as_str())
                .collect::<Vec<_>>(),
            [
                "kafka_produce:orders.created",
                "kafka_consume:billing.invoices"
            ]
        );
        let invoice = &out.operations[1];
        assert_eq!(invoice.name, "receiveInvoice");
        assert_eq!(
            names(invoice.response.as_ref().unwrap()),
            ["invoiceId", "amount"],
            "цепочка $ref канал → компонент"
        );
        let created = &out.operations[0];
        assert_eq!(created.summary, "Emit order created");
        assert_eq!(names(created.request.as_ref().unwrap()), ["orderId"]);
        assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    }

    #[test]
    fn other_documents_are_errors() {
        let read = |_: &str| None;
        assert!(
            parse(
                &ContractInput {
                    path: "x.yaml",
                    bytes: b"openapi: 3.0.0\n",
                    read: &read
                },
                &opts()
            )
            .is_err()
        );
        assert!(
            parse(
                &ContractInput {
                    path: "x.yaml",
                    bytes: b"asyncapi: 1.2.0\n",
                    read: &read
                },
                &opts()
            )
            .is_err()
        );
    }
}
