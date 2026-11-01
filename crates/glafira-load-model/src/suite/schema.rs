//! Типы сьюта и их разбор (спека L2 §3.1, §3.4).

use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer, Serialize};

use crate::registry::OpKind;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Suite {
    pub format: u32,
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub secrets: Vec<String>,
    /// Короткая форма `name: path` читается как `{ path, mode: shared }`; пишется всегда полная.
    #[serde(
        default,
        deserialize_with = "data_sets",
        skip_serializing_if = "BTreeMap::is_empty"
    )]
    pub data: BTreeMap<String, DataSet>,
    #[serde(default)]
    pub defaults: Defaults,
    pub scenarios: Vec<Scenario>,
}

/// Набор данных в YAML: строка пути или полная форма.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DataSetSpec {
    Path(String),
    Full(DataSet),
}

impl From<DataSetSpec> for DataSet {
    fn from(spec: DataSetSpec) -> Self {
        match spec {
            DataSetSpec::Path(path) => DataSet {
                path,
                mode: DataMode::default(),
            },
            DataSetSpec::Full(d) => d,
        }
    }
}

fn data_sets<'de, D: Deserializer<'de>>(d: D) -> Result<BTreeMap<String, DataSet>, D::Error> {
    let raw: BTreeMap<String, DataSetSpec> = BTreeMap::deserialize(d)?;
    Ok(raw.into_iter().map(|(k, v)| (k, v.into())).collect())
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DataSet {
    /// Относительно папки проекта, внутри неё.
    pub path: String,
    #[serde(default)]
    pub mode: DataMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataMode {
    /// Агенты делят строки файла по весам.
    Slice,
    /// У каждого агента весь файл.
    #[default]
    Shared,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Defaults {
    pub http: HttpDefaults,
    /// Параметры `rdkafka` продюсера и потребителя.
    pub kafka: BTreeMap<String, String>,
    pub arrival: Arrival,
    /// Потолок одновременных итераций на агента.
    pub max_in_flight: u32,
}

pub const DEFAULT_TIMEOUT_MS: u64 = 5000;
pub const DEFAULT_MAX_IN_FLIGHT: u32 = 1000;
pub const DEFAULT_CONSUME_TIMEOUT_MS: u64 = 10000;

impl Default for Defaults {
    fn default() -> Self {
        Self {
            http: HttpDefaults::default(),
            kafka: BTreeMap::new(),
            arrival: Arrival::default(),
            max_in_flight: DEFAULT_MAX_IN_FLIGHT,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct HttpDefaults {
    pub timeout_ms: u64,
    pub headers: BTreeMap<String, String>,
}

impl Default for HttpDefaults {
    fn default() -> Self {
        Self {
            timeout_ms: DEFAULT_TIMEOUT_MS,
            headers: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Arrival {
    #[default]
    Poisson,
    Constant,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    pub id: String,
    /// Ведущая операция; по умолчанию — операция первого шага.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drives: Option<String>,
    #[serde(default)]
    pub on_fail: OnFail,
    pub steps: Vec<Step>,
}

impl Scenario {
    /// Ведущая операция: `drives`, иначе операция первого шага; у сценария без шагов — пусто.
    pub fn drives(&self) -> &str {
        match &self.drives {
            Some(d) => d,
            None => self
                .steps
                .first()
                .map(|s| s.action.operation())
                .unwrap_or(""),
        }
    }

    /// Идентификаторы шагов; без `id` — `step<N>` (с 1).
    pub fn step_ids(&self) -> Vec<String> {
        self.steps
            .iter()
            .enumerate()
            .map(|(i, s)| s.id.clone().unwrap_or_else(|| format!("step{}", i + 1)))
            .collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnFail {
    #[default]
    Stop,
    Continue,
}

/// Шаг: ровно один из ключей `http`, `kafka_produce`, `kafka_consume`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(flatten)]
    pub action: Action,
    #[serde(default, skip_serializing_if = "Expect::is_empty")]
    pub expect: Expect,
    /// Имя → JSONPath по телу ответа или сообщения.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extract: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Http(HttpStep),
    KafkaProduce(ProduceStep),
    KafkaConsume(ConsumeStep),
}

impl Action {
    pub fn operation(&self) -> &str {
        match self {
            Action::Http(s) => &s.operation,
            Action::KafkaProduce(s) => &s.operation,
            Action::KafkaConsume(s) => &s.operation,
        }
    }

    pub fn kind(&self) -> OpKind {
        match self {
            Action::Http(_) => OpKind::Http,
            Action::KafkaProduce(_) => OpKind::KafkaProduce,
            Action::KafkaConsume(_) => OpKind::KafkaConsume,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HttpStep {
    pub operation: String,
    /// Параметры пути операции.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub path: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub query: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
    /// YAML, который уходит как JSON.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<serde_json::Value>,
    /// Тело как есть.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProduceStep {
    pub operation: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value_text: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub encoding: Encoding,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Encoding {
    #[default]
    Json,
    /// Схема из контракта, id — из Schema Registry стенда.
    Avro,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConsumeStep {
    pub operation: String,
    #[serde(rename = "match", default, skip_serializing_if = "Match::is_empty")]
    pub match_: Match,
    #[serde(default = "default_consume_timeout")]
    pub timeout_ms: u64,
}

fn default_consume_timeout() -> u64 {
    DEFAULT_CONSUME_TIMEOUT_MS
}

/// Чего ждать в топике: ключ и/или значения по JSONPath.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields, default)]
pub struct Match {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub jsonpath: BTreeMap<String, String>,
}

impl Match {
    pub fn is_empty(&self) -> bool {
        self.key.is_none() && self.jsonpath.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields, default)]
pub struct Expect {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<StatusExpect>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub jsonpath: BTreeMap<String, Matcher>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema: Option<SchemaExpect>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_ms: Option<u64>,
}

impl Expect {
    pub fn is_empty(&self) -> bool {
        self.status.is_none()
            && self.jsonpath.is_empty()
            && self.schema.is_none()
            && self.max_ms.is_none()
    }
}

/// `201`, `[200, 201]` или класс `2xx`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum StatusExpect {
    One(u16),
    Many(Vec<u16>),
    Class(String),
}

impl StatusExpect {
    /// Класс вида `2xx`…`5xx`.
    pub fn is_valid_class(class: &str) -> bool {
        let b = class.as_bytes();
        b.len() == 3 && (b'1'..=b'5').contains(&b[0]) && &b[1..] == b"xx"
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SchemaExpect {
    Response,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Matcher {
    Exists(ExistsTag),
    Equals { equals: serde_json::Value },
    Matches { matches: String },
    Gt { gt: f64 },
    Lt { lt: f64 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExistsTag {
    Exists,
}

/// Ошибка разбора сьюта: текст и строка, если YAML её знает.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct SuiteError {
    pub line: Option<u32>,
    pub message: String,
}

/// Разбор YAML; короткая форма `data` приводится к полной.
pub fn parse(text: &str) -> Result<Suite, SuiteError> {
    serde_yaml_ng::from_str(text).map_err(|e| SuiteError {
        line: e.location().map(|l| l.line() as u32),
        message: e.to_string(),
    })
}

/// Сьют пишется из согласованной структуры: ошибка сериализации — баг, не ввод.
pub fn to_yaml(suite: &Suite) -> String {
    serde_yaml_ng::to_string(suite).expect("suite serializes")
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) const SAMPLE: &str =
        include_str!("../../../../dev/fixtures/suites/orders/suite.yaml");

    #[test]
    fn parses_the_sample_suite_with_defaults_and_short_forms() {
        let s = parse(SAMPLE).unwrap();
        assert_eq!((s.format, s.name.as_str()), (1, "orders"));
        assert_eq!(s.secrets, vec!["API_TOKEN"]);
        assert_eq!(
            s.data["users"],
            DataSet {
                path: "data/users.csv".into(),
                mode: DataMode::Slice
            }
        );
        assert_eq!(
            s.data["skus"],
            DataSet {
                path: "data/skus.jsonl".into(),
                mode: DataMode::Shared
            },
            "короткая форма"
        );
        assert_eq!(s.defaults.http.headers["Accept"], "application/json");
        assert_eq!(s.defaults.kafka["acks"], "all");
        assert_eq!(
            (s.defaults.arrival, s.defaults.max_in_flight),
            (Arrival::Poisson, 1000)
        );
        let sc = &s.scenarios[0];
        assert_eq!(sc.drives(), "http:orders:POST /v1/orders");
        assert_eq!(sc.on_fail, OnFail::Stop);
        assert_eq!(sc.step_ids(), vec!["create", "get", "step3", "step4"]);
        let Action::Http(create) = &sc.steps[0].action else {
            panic!("http")
        };
        assert_eq!(
            create.headers["Authorization"],
            "Bearer {{secret API_TOKEN}}"
        );
        assert_eq!(
            create.body.as_ref().unwrap()["items"][0]["qty"],
            serde_json::json!("{{int 1 5}}")
        );
        assert_eq!(sc.steps[0].expect.status, Some(StatusExpect::One(201)));
        assert_eq!(
            sc.steps[0].expect.jsonpath["$.id"],
            Matcher::Exists(ExistsTag::Exists)
        );
        assert_eq!(sc.steps[0].extract["order_id"], "$.id");
        assert_eq!(sc.steps[1].expect.schema, Some(SchemaExpect::Response));
        assert_eq!(sc.steps[1].expect.max_ms, Some(800));
        let Action::KafkaConsume(c) = &sc.steps[2].action else {
            panic!("consume")
        };
        assert_eq!(
            (c.match_.key.as_deref(), c.timeout_ms),
            (Some("{{steps.create.order_id}}"), 10000)
        );
        let Action::KafkaProduce(p) = &sc.steps[3].action else {
            panic!("produce")
        };
        assert_eq!(
            (p.encoding, p.key.as_deref()),
            (Encoding::Json, Some("{{steps.create.order_id}}"))
        );
        assert_eq!(sc.steps[3].action.kind(), OpKind::KafkaProduce);
    }

    #[test]
    fn defaults_apply_when_sections_are_missing() {
        let s = parse("format: 1\nname: x\nscenarios:\n  - id: a\n    steps:\n      - kafka_consume: { operation: \"kafka_consume:t\" }\n").unwrap();
        assert_eq!(s.defaults, Defaults::default());
        assert_eq!(s.defaults.http.timeout_ms, 5000);
        let Action::KafkaConsume(c) = &s.scenarios[0].steps[0].action else {
            panic!()
        };
        assert_eq!(c.timeout_ms, 10000);
        assert!(c.match_.is_empty());
        assert_eq!(s.scenarios[0].drives(), "kafka_consume:t");
        assert!(s.scenarios[0].steps[0].expect.is_empty());
    }

    #[test]
    fn unknown_fields_and_wrong_types_are_errors_with_a_line() {
        let e = parse("format: 1\nname: x\nscenarios: []\nbogus: 1\n").unwrap_err();
        assert!(e.message.contains("unknown field `bogus`"), "{e}");
        assert_eq!(e.line, Some(4));
        let e = parse("format: 1\nname: x\nscenarios:\n  - id: a\n    steps:\n      - http: { operation: o }\n        expct: { status: 200 }\n").unwrap_err();
        assert!(e.message.contains("unknown field `expct`"), "{e}");
        assert_eq!(e.line, Some(6), "строка шага");
        let e = parse("format: 1\nname: x\nscenarios:\n  - id: a\n    steps:\n      - http: { operation: o, timeout_ms: soon }\n").unwrap_err();
        assert!(e.message.contains("expected u64"), "{e}");
        assert_eq!(e.line, Some(6));
        let e = parse("format: 1\nname: x\nscenarios:\n  - id: a\n    steps:\n      - id: s\n")
            .unwrap_err();
        assert!(e.message.contains("no variant of enum Action"), "{e}");
        let e = parse("format: 1\nname: x\nscenarios:\n  - id: a\n    steps:\n      - http: { operation: o }\n        kafka_produce: { operation: p }\n").unwrap_err();
        assert!(
            e.message.contains("unknown field `kafka_produce`"),
            "два действия в шаге: {e}"
        );
        let e = parse("format: 1\nname: x\ndata:\n  u: { path: a, mod: slice }\nscenarios: []\n")
            .unwrap_err();
        assert!(e.message.contains("did not match any variant"), "{e}");
        assert_eq!(e.line, Some(4));
    }

    #[test]
    fn matchers_and_status_forms() {
        let s = parse("format: 1\nname: x\nscenarios:\n  - id: a\n    steps:\n      - http: { operation: o }\n        expect:\n          status: [200, 204]\n          jsonpath: { \"$.a\": exists, \"$.b\": { equals: 5 }, \"$.c\": { matches: \"^x\" }, \"$.d\": { gt: 1.5 }, \"$.e\": { lt: 2 } }\n      - http: { operation: o }\n        expect: { status: 2xx }\n").unwrap();
        let e = &s.scenarios[0].steps[0].expect;
        assert_eq!(e.status, Some(StatusExpect::Many(vec![200, 204])));
        assert_eq!(
            e.jsonpath["$.b"],
            Matcher::Equals {
                equals: serde_json::json!(5)
            }
        );
        assert_eq!(
            e.jsonpath["$.c"],
            Matcher::Matches {
                matches: "^x".into()
            }
        );
        assert_eq!(e.jsonpath["$.d"], Matcher::Gt { gt: 1.5 });
        assert_eq!(e.jsonpath["$.e"], Matcher::Lt { lt: 2.0 });
        assert_eq!(
            s.scenarios[0].steps[1].expect.status,
            Some(StatusExpect::Class("2xx".into()))
        );
        assert!(
            StatusExpect::is_valid_class("4xx")
                && !StatusExpect::is_valid_class("6xx")
                && !StatusExpect::is_valid_class("2x")
        );
        let e = parse("format: 1\nname: x\nscenarios:\n  - id: a\n    steps:\n      - http: { operation: o }\n        expect: { jsonpath: { \"$.a\": { eq: 1 } } }\n").unwrap_err();
        assert!(e.message.contains("Matcher"), "{e}");
    }

    #[test]
    fn yaml_round_trip_is_stable_and_skips_defaults() {
        let s = parse(SAMPLE).unwrap();
        let text = to_yaml(&s);
        assert_eq!(parse(&text).unwrap(), s);
        assert_eq!(to_yaml(&parse(&text).unwrap()), text);
        assert!(text.contains("\n- id: order-flow\n"), "{text}");
        assert!(
            text.contains("\n  - kafka_consume:\n"),
            "ключ действия — вид шага: {text}"
        );
        assert!(
            !text.contains("extract: {}") && !text.contains("id: null"),
            "{text}"
        );
        assert!(
            text.contains("mode: shared"),
            "короткая форма пишется полной: {text}"
        );
    }
}
