//! Манифест проекта `glafira-load.yaml` (спека L1 §3.2–§3.3).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub const MANIFEST_FILE: &str = "glafira-load.yaml";
pub const FORMAT: u32 = 1;
/// Максимальная длина `id` сервиса и стенда.
pub const MAX_ID_LEN: usize = 64;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ContractKind {
    Openapi,
    Proto,
    Asyncapi,
    Schema,
    Sql,
    Spring,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContractRef {
    pub kind: ContractKind,
    /// Относительно клона сервиса, разделитель `/`.
    pub path: String,
    /// Каталоги для `import` (proto), относительно клона.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub include: Vec<String>,
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub enabled: bool,
}

fn yes() -> bool {
    true
}
fn is_true(b: &bool) -> bool {
    *b
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    #[default]
    Produce,
    Consume,
}

/// Схема топика без AsyncAPI (решение плана 6): путь к файлу или путь с направлением.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum TopicSchema {
    Path(String),
    Full {
        schema: String,
        #[serde(default)]
        direction: Direction,
    },
}

impl TopicSchema {
    pub fn schema(&self) -> &str {
        match self {
            TopicSchema::Path(p) => p,
            TopicSchema::Full { schema, .. } => schema,
        }
    }
    pub fn direction(&self) -> Direction {
        match self {
            TopicSchema::Path(_) => Direction::Produce,
            TopicSchema::Full { direction, .. } => *direction,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct KafkaMap {
    #[serde(default)]
    pub topics: BTreeMap<String, TopicSchema>,
}

impl KafkaMap {
    pub fn is_empty(&self) -> bool {
        self.topics.is_empty()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Service {
    pub id: String,
    #[serde(default)]
    pub name: String,
    /// Клон git: относительно проекта или абсолютный.
    pub path: String,
    /// Пусто — автопоиск по маскам (спека §5.1).
    #[serde(default)]
    pub contracts: Vec<ContractRef>,
    #[serde(default, skip_serializing_if = "KafkaMap::is_empty")]
    pub kafka: KafkaMap,
    /// Неизвестные поля переживают перезапись.
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EnvKind {
    Dev,
    Test,
    Stage,
    Prod,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Environment {
    pub id: String,
    pub kind: EnvKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kafka: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grpc: Option<String>,
    /// Строка подключения без пароля; пароль — L2.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sql: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Manifest {
    pub format: u32,
    pub name: String,
    #[serde(default)]
    pub services: Vec<Service>,
    #[serde(default)]
    pub environments: Vec<Environment>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

impl Manifest {
    pub fn new(name: &str) -> Self {
        Self {
            format: FORMAT,
            name: name.into(),
            services: vec![],
            environments: vec![],
            extra: BTreeMap::new(),
        }
    }

    pub fn service(&self, id: &str) -> Option<&Service> {
        self.services.iter().find(|s| s.id == id)
    }
}

/// Нарушение правил §3.3. Отдаётся фронтенду как есть: текст подбирается по `kind`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ManifestProblem {
    BadFormat { format: u32 },
    EmptyName,
    BadServiceId { id: String },
    DuplicateServiceId { id: String },
    EmptyServicePath { id: String },
    BadEnvironmentId { id: String },
    DuplicateEnvironmentId { id: String },
}

/// `[a-z0-9-]{1,64}`.
pub fn is_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= MAX_ID_LEN
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// Проверки без обращения к диску; существование путей проверяет приложение.
pub fn validate(m: &Manifest) -> Vec<ManifestProblem> {
    let mut out = Vec::new();
    if m.format != FORMAT {
        out.push(ManifestProblem::BadFormat { format: m.format });
    }
    if m.name.trim().is_empty() {
        out.push(ManifestProblem::EmptyName);
    }
    let mut seen = std::collections::BTreeSet::new();
    for s in &m.services {
        if !seen.insert(s.id.as_str()) {
            out.push(ManifestProblem::DuplicateServiceId { id: s.id.clone() });
        } else if !is_id(&s.id) {
            out.push(ManifestProblem::BadServiceId { id: s.id.clone() });
        }
        if s.path.trim().is_empty() {
            out.push(ManifestProblem::EmptyServicePath { id: s.id.clone() });
        }
    }
    let mut seen = std::collections::BTreeSet::new();
    for e in &m.environments {
        if !seen.insert(e.id.as_str()) {
            out.push(ManifestProblem::DuplicateEnvironmentId { id: e.id.clone() });
        } else if !is_id(&e.id) {
            out.push(ManifestProblem::BadEnvironmentId { id: e.id.clone() });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
format: 1
name: orders-platform
services:
  - id: orders
    name: Orders Service
    path: ../orders-service
    contracts:
      - kind: openapi
        path: api/openapi.yaml
      - kind: proto
        path: proto
        include: [proto, third_party]
        enabled: false
    kafka:
      topics:
        orders.created: schemas/orders-created.avsc
        orders.paid: { schema: schemas/paid.avsc, direction: consume }
    team: platform
environments:
  - id: test
    kind: test
    http: https://orders.test.local
    kafka: kafka-1.test.local:9092
"#;

    #[test]
    fn reads_manifest_keeps_unknown_fields_and_writes_them_back() {
        let m: Manifest = crate::from_yaml(SAMPLE).unwrap();
        assert_eq!(m.name, "orders-platform");
        assert_eq!(m.services[0].contracts[1].kind, ContractKind::Proto);
        assert!(!m.services[0].contracts[1].enabled);
        assert!(
            m.services[0].contracts[0].enabled,
            "enabled по умолчанию true"
        );
        assert_eq!(
            m.services[0].kafka.topics["orders.paid"],
            TopicSchema::Full {
                schema: "schemas/paid.avsc".into(),
                direction: Direction::Consume
            }
        );
        assert_eq!(m.services[0].extra["team"], serde_json::json!("platform"));
        assert_eq!(m.environments[0].kind, EnvKind::Test);
        let again: Manifest = crate::from_yaml(&crate::to_yaml(&m)).unwrap();
        assert_eq!(again, m, "неизвестное поле team пережило запись");
        assert!(validate(&m).is_empty());
    }

    #[test]
    fn validation_finds_bad_and_duplicate_ids() {
        let mut m: Manifest = crate::from_yaml(SAMPLE).unwrap();
        m.services.push(Service {
            id: "orders".into(),
            name: String::new(),
            path: "x".into(),
            contracts: vec![],
            kafka: KafkaMap::default(),
            extra: Default::default(),
        });
        m.services.push(Service {
            id: "Bad Id".into(),
            name: String::new(),
            path: String::new(),
            contracts: vec![],
            kafka: KafkaMap::default(),
            extra: Default::default(),
        });
        m.environments.push(Environment {
            id: "test".into(),
            kind: EnvKind::Prod,
            http: None,
            kafka: None,
            grpc: None,
            sql: None,
            extra: Default::default(),
        });
        m.name = "  ".into();
        m.format = 2;
        let problems = validate(&m);
        assert_eq!(
            problems,
            vec![
                ManifestProblem::BadFormat { format: 2 },
                ManifestProblem::EmptyName,
                ManifestProblem::DuplicateServiceId {
                    id: "orders".into()
                },
                ManifestProblem::BadServiceId {
                    id: "Bad Id".into()
                },
                ManifestProblem::EmptyServicePath {
                    id: "Bad Id".into()
                },
                ManifestProblem::DuplicateEnvironmentId { id: "test".into() },
            ]
        );
    }

    #[test]
    fn topic_schema_direction_defaults_to_produce() {
        let t: TopicSchema = crate::from_yaml("schemas/a.avsc").unwrap();
        assert_eq!(t.schema(), "schemas/a.avsc");
        assert_eq!(t.direction(), Direction::Produce);
    }

    #[test]
    fn bad_yaml_reports_a_line() {
        let e = crate::from_yaml::<Manifest>("format: 1\nname: x\nservices: [\n").unwrap_err();
        assert!(e.line.is_some(), "{e:?}");
    }
}
