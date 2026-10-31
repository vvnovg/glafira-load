//! Реестр операций `registry/operations.yaml` (спека L1 §6.1).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::schema::{SchemaSummary, fingerprint_of};

pub const REGISTRY_FILE: &str = "registry/operations.yaml";
pub const SCANS_DIR: &str = "registry/scans";
pub const FORMAT: u32 = 1;

fn is_false(b: &bool) -> bool {
    !*b
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum OpKind {
    Http,
    KafkaProduce,
    KafkaConsume,
    Grpc,
    Sql,
}

impl OpKind {
    pub fn as_str(self) -> &'static str {
        match self {
            OpKind::Http => "http",
            OpKind::KafkaProduce => "kafka_produce",
            OpKind::KafkaConsume => "kafka_consume",
            OpKind::Grpc => "grpc",
            OpKind::Sql => "sql",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Address {
    Http { method: String, path: String },
    Kafka { topic: String },
    Grpc { service: String, method: String },
    Sql { name: String },
}

/// Адрес одной строкой: для `id`, диффов и заголовков.
pub fn address_label(a: &Address) -> String {
    match a {
        Address::Http { method, path } => format!("{} {path}", method.to_ascii_uppercase()),
        Address::Kafka { topic } => topic.clone(),
        Address::Grpc { service, method } => format!("{service}/{method}"),
        Address::Sql { name } => name.clone(),
    }
}

/// Устойчивый `id` (Global Constraints). Пара вид/адрес, которой нет в правилах, даёт общий вид
/// `<kind>:<service>:<адрес>` — ошибка импортёра не должна ронять разбор.
pub fn operation_id(service: &str, kind: OpKind, address: &Address) -> String {
    match (kind, address) {
        (OpKind::Http, Address::Http { .. }) => {
            format!("http:{service}:{}", address_label(address))
        }
        (OpKind::KafkaProduce, Address::Kafka { topic }) => format!("kafka_produce:{topic}"),
        (OpKind::KafkaConsume, Address::Kafka { topic }) => format!("kafka_consume:{topic}"),
        (OpKind::Grpc, Address::Grpc { .. }) => {
            format!("grpc:{service}:{}", address_label(address))
        }
        (OpKind::Sql, Address::Sql { name }) => format!("sql:{service}:{name}"),
        (kind, address) => format!("{}:{service}:{}", kind.as_str(), address_label(address)),
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    Openapi,
    Proto,
    Asyncapi,
    Schema,
    Sql,
    Spring,
    Manual,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Source {
    pub kind: SourceKind,
    /// Относительно клона сервиса; у `manual` — пусто.
    #[serde(default)]
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
}

impl Source {
    pub fn manual() -> Self {
        Self {
            kind: SourceKind::Manual,
            path: String::new(),
            line: None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Active,
    Removed,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Streaming {
    Client,
    Server,
    Bidi,
}

/// Версия сервиса, в которой операция замечена.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct Seen {
    pub version: String,
    pub commit: String,
    /// RFC 3339, UTC.
    pub at: String,
}

/// Ручные поля: переживают переразбор.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct Manual {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub notes: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub owner: String,
}

impl Manual {
    pub fn is_empty(&self) -> bool {
        self.tags.is_empty() && self.notes.is_empty() && self.owner.is_empty()
    }
}

/// Операция из импортёра: всё, что известно из контракта. Остальное добавляет слияние.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OperationDraft {
    pub id: String,
    pub service: String,
    pub kind: OpKind,
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub summary: String,
    pub address: Address,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request: Option<SchemaSummary>,
    /// Ответ 2xx у HTTP, ответ rpc у gRPC, сообщение у Kafka.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response: Option<SchemaSummary>,
    /// Остальные ответы HTTP по коду.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub responses: BTreeMap<String, SchemaSummary>,
    pub source: Source,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub streaming: Option<Streaming>,
    /// Топик взят из имени файла схемы, а не из манифеста.
    #[serde(default, skip_serializing_if = "is_false")]
    pub topic_guessed: bool,
}

impl OperationDraft {
    /// Черновик с `id` по правилам и именем из адреса; импортёр уточняет имя и схемы.
    pub fn new(service: &str, kind: OpKind, address: Address, source: Source) -> Self {
        Self {
            id: operation_id(service, kind, &address),
            service: service.into(),
            kind,
            name: address_label(&address),
            summary: String::new(),
            address,
            request: None,
            response: None,
            responses: BTreeMap::new(),
            source,
            streaming: None,
            topic_guessed: false,
        }
    }

    /// Отпечаток: адрес, стриминг и канонические схемы. Имя, описание и источник не входят.
    pub fn fingerprint(&self) -> String {
        fingerprint_of(&serde_json::json!({
            "a": self.address,
            "s": self.streaming,
            "q": self.request.as_ref().map(SchemaSummary::canonical),
            "p": self.response.as_ref().map(SchemaSummary::canonical),
            "r": self.responses.iter().map(|(k, v)| (k.clone(), v.canonical())).collect::<BTreeMap<_, _>>(),
        }))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Operation {
    #[serde(flatten)]
    pub draft: OperationDraft,
    pub fingerprint: String,
    pub status: Status,
    pub first_seen: Seen,
    pub last_seen: Seen,
    #[serde(default, skip_serializing_if = "Manual::is_empty")]
    pub manual: Manual,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Registry {
    pub format: u32,
    #[serde(default)]
    pub operations: Vec<Operation>,
}

impl Default for Registry {
    fn default() -> Self {
        Self {
            format: FORMAT,
            operations: Vec::new(),
        }
    }
}

impl Registry {
    pub fn get(&self, id: &str) -> Option<&Operation> {
        self.operations.iter().find(|o| o.draft.id == id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut Operation> {
        self.operations.iter_mut().find(|o| o.draft.id == id)
    }

    /// Порядок файла — по сервису, затем по `id`: дифф в git читается.
    pub fn sort(&mut self) {
        self.operations.sort_by(|a, b| {
            (a.draft.service.as_str(), a.draft.id.as_str())
                .cmp(&(b.draft.service.as_str(), b.draft.id.as_str()))
        });
    }

    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.operations.len();
        self.operations.retain(|o| o.draft.id != id);
        before != self.operations.len()
    }

    /// Отпечатки активных операций: для снимка разбора.
    pub fn fingerprints(&self) -> BTreeMap<String, String> {
        self.operations
            .iter()
            .filter(|o| o.status == Status::Active)
            .map(|o| (o.draft.id.clone(), o.fingerprint.clone()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seen() -> Seen {
        Seen {
            version: "v1".into(),
            commit: "abc".into(),
            at: "2026-10-31T10:00:00Z".into(),
        }
    }

    #[test]
    fn ids_follow_the_rules() {
        assert_eq!(
            operation_id(
                "orders",
                OpKind::Http,
                &Address::Http {
                    method: "post".into(),
                    path: "/v1/orders".into()
                }
            ),
            "http:orders:POST /v1/orders"
        );
        assert_eq!(
            operation_id(
                "orders",
                OpKind::KafkaProduce,
                &Address::Kafka {
                    topic: "orders.created".into()
                }
            ),
            "kafka_produce:orders.created"
        );
        assert_eq!(
            operation_id(
                "orders",
                OpKind::KafkaConsume,
                &Address::Kafka {
                    topic: "orders.created".into()
                }
            ),
            "kafka_consume:orders.created"
        );
        assert_eq!(
            operation_id(
                "orders",
                OpKind::Grpc,
                &Address::Grpc {
                    service: "orders.v1.OrderService".into(),
                    method: "Create".into()
                }
            ),
            "grpc:orders:orders.v1.OrderService/Create"
        );
        assert_eq!(
            operation_id(
                "billing",
                OpKind::Sql,
                &Address::Sql {
                    name: "invoices.insert".into()
                }
            ),
            "sql:billing:invoices.insert"
        );
        assert_eq!(
            operation_id("x", OpKind::Sql, &Address::Kafka { topic: "t".into() }),
            "sql:x:t",
            "несогласованная пара не паникует"
        );
    }

    #[test]
    fn fingerprint_depends_on_schemas_and_address_only() {
        let mut a = OperationDraft::new(
            "orders",
            OpKind::Http,
            Address::Http {
                method: "POST".into(),
                path: "/v1/orders".into(),
            },
            Source {
                kind: SourceKind::Openapi,
                path: "api.yaml".into(),
                line: Some(3),
            },
        );
        a.request = Some(SchemaSummary::leaf("body", "object"));
        let mut b = a.clone();
        b.name = "createOrder".into();
        b.summary = "Create".into();
        b.source.line = Some(99);
        assert_eq!(a.fingerprint(), b.fingerprint());
        b.request = Some(SchemaSummary::leaf("body", "string"));
        assert_ne!(a.fingerprint(), b.fingerprint());
    }

    #[test]
    fn registry_round_trips_through_yaml_flat() {
        let draft = OperationDraft::new(
            "orders",
            OpKind::Http,
            Address::Http {
                method: "GET".into(),
                path: "/v1/orders/{id}".into(),
            },
            Source {
                kind: SourceKind::Openapi,
                path: "api.yaml".into(),
                line: None,
            },
        );
        let fp = draft.fingerprint();
        let mut r = Registry::default();
        r.operations.push(Operation {
            draft,
            fingerprint: fp,
            status: Status::Active,
            first_seen: seen(),
            last_seen: seen(),
            manual: Manual {
                tags: vec!["critical".into()],
                ..Default::default()
            },
        });
        let text = crate::to_yaml(&r);
        // serde_yaml_ng 0.10 не отступает элементы списка и не кавычит безопасные скаляры
        assert!(
            text.contains("\n- id: http:orders:GET /v1/orders/{id}\n"),
            "{text}"
        );
        assert!(
            text.contains("\n  address:\n    type: http\n"),
            "поля черновика лежат плоско: {text}"
        );
        assert!(!text.contains("summary:"), "пустые поля не пишутся");
        let again: Registry = crate::from_yaml(&text).unwrap();
        assert_eq!(again, r);
        assert_eq!(again.fingerprints().len(), 1);
    }

    #[test]
    fn sort_is_by_service_then_id() {
        let mk = |service: &str, path: &str| {
            let d = OperationDraft::new(
                service,
                OpKind::Http,
                Address::Http {
                    method: "GET".into(),
                    path: path.into(),
                },
                Source::manual(),
            );
            Operation {
                fingerprint: d.fingerprint(),
                draft: d,
                status: Status::Active,
                first_seen: seen(),
                last_seen: seen(),
                manual: Manual::default(),
            }
        };
        let mut r = Registry {
            format: FORMAT,
            operations: vec![mk("b", "/z"), mk("a", "/y"), mk("b", "/a")],
        };
        r.sort();
        assert_eq!(
            r.operations
                .iter()
                .map(|o| o.draft.id.as_str())
                .collect::<Vec<_>>(),
            ["http:a:GET /y", "http:b:GET /a", "http:b:GET /z"]
        );
    }
}
