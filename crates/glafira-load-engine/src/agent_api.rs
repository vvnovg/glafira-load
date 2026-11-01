//! Типы протокола контроллер ↔ агент (спека L2 §4.3–§4.4): JSON в camelCase.

use std::collections::BTreeMap;

use glafira_load_model::profile::Point;
use glafira_load_model::schema::SchemaSummary;
use glafira_load_model::suite::Arrival;
use serde::{Deserialize, Serialize};

pub const API_VERSION: &str = "v1";

/// Ответ `GET /v1/info`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentInfo {
    pub version: String,
    pub os: String,
    pub arch: String,
    pub cpus: usize,
    /// `run_id` текущего прогона, если агент занят.
    #[serde(default)]
    pub busy: Option<String>,
}

/// Адреса стенда, которые агент подставляет в `env.*`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvAddresses {
    #[serde(default)]
    pub http: Option<String>,
    #[serde(default)]
    pub kafka: Option<String>,
    #[serde(default)]
    pub schema_registry: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Run,
    Smoke,
}

/// `run.json` бандла `POST /v1/runs`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunRequest {
    pub run_id: String,
    pub agent_id: String,
    pub suite_name: String,
    pub controller_version: String,
    pub suite_format: u32,
    pub env: EnvAddresses,
    #[serde(default)]
    pub secrets: Vec<String>,
    /// Цели операций, уже умноженные на долю агента.
    #[serde(default)]
    pub targets: BTreeMap<String, f64>,
    #[serde(default)]
    pub timeline: Vec<Point>,
    pub total_minutes: f64,
    pub max_in_flight: u32,
    pub arrival: Arrival,
    pub mode: Mode,
    #[serde(default)]
    pub kafka: BTreeMap<String, String>,
    /// Схемы ответов операций (`expect.schema: response`): id операции → сжатая схема реестра.
    /// Контроллер берёт их из реестра проекта; у агента реестра нет.
    #[serde(default)]
    pub response_schemas: BTreeMap<String, SchemaSummary>,
    /// Тексты `.avsc` для `kafka_produce` с `encoding: avro`: id операции → схема.
    #[serde(default)]
    pub avro_schemas: BTreeMap<String, String>,
}

/// Метрики операции за одну секунду.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpTick {
    pub id: String,
    pub n: u64,
    #[serde(default)]
    pub err: BTreeMap<String, u64>,
    pub dropped: u64,
    pub p50: f64,
    pub p90: f64,
    pub p95: f64,
    pub p99: f64,
    pub max: f64,
    pub bytes_in: u64,
    pub bytes_out: u64,
    #[serde(default)]
    pub lag: Option<i64>,
}

/// Метрики сценария за одну секунду (итерации целиком).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScenarioTick {
    pub id: String,
    pub n: u64,
    #[serde(default)]
    pub err: BTreeMap<String, u64>,
    pub dropped: u64,
    pub p50: f64,
    pub p95: f64,
    pub p99: f64,
    pub max: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tick {
    /// Секунда от старта прогона.
    pub t: u64,
    #[serde(default)]
    pub ops: Vec<OpTick>,
    #[serde(default)]
    pub scenarios: Vec<ScenarioTick>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PhaseEvent {
    pub t: u64,
    /// Индекс отрезка таймлайна.
    pub index: usize,
    pub pct: f64,
}

/// Пример ошибки шага: запрос и ответ уже замаскированы и обрезаны.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorSample {
    pub t: u64,
    pub code: String,
    pub operation: String,
    pub scenario: String,
    pub step: String,
    pub agent: String,
    pub message: String,
    #[serde(default)]
    pub request: Option<String>,
    #[serde(default)]
    pub response: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Completed,
    Stopped,
    Incomplete,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpSummary {
    pub id: String,
    pub n: u64,
    pub rps_avg: f64,
    pub rps_peak: f64,
    #[serde(default)]
    pub err: BTreeMap<String, u64>,
    pub err_pct: f64,
    pub dropped: u64,
    pub p50: f64,
    pub p90: f64,
    pub p95: f64,
    pub p99: f64,
    pub max: f64,
    #[serde(default)]
    pub target: Option<f64>,
    #[serde(default)]
    pub achieved_pct: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScenarioSummary {
    pub id: String,
    pub n: u64,
    #[serde(default)]
    pub err: BTreeMap<String, u64>,
    pub p50: f64,
    pub p95: f64,
    pub p99: f64,
    pub max: f64,
}

/// Сводка агента (событие `done`) и итог прогона после слияния.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSummary {
    pub status: RunStatus,
    #[serde(default)]
    pub ops: Vec<OpSummary>,
    #[serde(default)]
    pub scenarios: Vec<ScenarioSummary>,
    pub seconds: u64,
}

/// Событие SSE `GET /v1/runs/{id}/events`; поле `type` — имя события.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    Tick(Tick),
    Phase(PhaseEvent),
    Error(ErrorSample),
    Done(AgentSummary),
    /// Буфер агента не покрыл `Last-Event-ID`: часть событий потеряна.
    Gap,
}

/// Тело ошибки агента (`401`, `409`, `412`, `422`, `426`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiError {
    pub code: String,
    pub message: String,
    #[serde(default)]
    pub names: Vec<String>,
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub controller: Option<String>,
}

/// Элемент `GET /v1/runs`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunListItem {
    pub run_id: String,
    pub status: RunStatus,
    pub started_at: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tick() -> Tick {
        Tick {
            t: 12,
            ops: vec![OpTick {
                id: "http:orders:POST /v1/orders".into(),
                n: 480,
                err: BTreeMap::from([("HTTP_5xx".to_string(), 3)]),
                dropped: 0,
                p50: 41.0,
                p90: 80.0,
                p95: 97.0,
                p99: 180.0,
                max: 412.0,
                bytes_in: 120_400,
                bytes_out: 51_200,
                lag: None,
            }],
            scenarios: vec![ScenarioTick {
                id: "order-flow".into(),
                n: 240,
                err: BTreeMap::new(),
                dropped: 1,
                p50: 90.0,
                p95: 200.0,
                p99: 300.0,
                max: 500.0,
            }],
        }
    }

    #[test]
    fn run_request_round_trips_in_camel_case() {
        let req = RunRequest {
            run_id: "r1".into(),
            agent_id: "gen-1".into(),
            suite_name: "orders".into(),
            controller_version: "0.2.0".into(),
            suite_format: 1,
            env: EnvAddresses {
                http: Some("http://127.0.0.1:1".into()),
                kafka: None,
                schema_registry: None,
            },
            secrets: vec!["API_TOKEN".into()],
            targets: BTreeMap::from([("http:orders:POST /v1/orders".to_string(), 50.0)]),
            timeline: vec![
                Point {
                    minute: 0.0,
                    pct: 0.0,
                },
                Point {
                    minute: 5.0,
                    pct: 100.0,
                },
            ],
            total_minutes: 5.0,
            max_in_flight: 1000,
            arrival: Arrival::Poisson,
            mode: Mode::Run,
            kafka: BTreeMap::new(),
            response_schemas: BTreeMap::from([(
                "http:orders:POST /v1/orders".to_string(),
                SchemaSummary::default(),
            )]),
            avro_schemas: BTreeMap::from([(
                "kafka_produce:orders.paid".to_string(),
                "{\"type\":\"record\"}".to_string(),
            )]),
        };
        let text = serde_json::to_string(&req).unwrap();
        assert!(text.contains("\"runId\":\"r1\""), "{text}");
        assert!(
            text.contains("\"responseSchemas\":{") && text.contains("\"avroSchemas\":{"),
            "{text}"
        );
        assert!(text.contains("\"maxInFlight\":1000"), "{text}");
        assert!(text.contains("\"arrival\":\"poisson\""), "{text}");
        assert!(text.contains("\"mode\":\"run\""), "{text}");
        assert!(text.contains("\"schemaRegistry\":null"), "{text}");
        let again: RunRequest = serde_json::from_str(&text).unwrap();
        assert_eq!(again, req);
        // Пустые карты и списки можно не присылать.
        let minimal: RunRequest = serde_json::from_str(
            r#"{"runId":"r","agentId":"a","suiteName":"s","controllerVersion":"0.2.0","suiteFormat":1,
                "env":{},"totalMinutes":1,"maxInFlight":10,"arrival":"constant","mode":"smoke"}"#,
        )
        .unwrap();
        assert_eq!(minimal.mode, Mode::Smoke);
        assert!(minimal.targets.is_empty() && minimal.timeline.is_empty());
        assert!(minimal.response_schemas.is_empty() && minimal.avro_schemas.is_empty());
    }

    #[test]
    fn events_are_tagged_by_type() {
        let cases: Vec<(Event, &str)> = vec![
            (Event::Tick(tick()), r#""type":"tick""#),
            (
                Event::Phase(PhaseEvent {
                    t: 60,
                    index: 1,
                    pct: 100.0,
                }),
                r#""type":"phase""#,
            ),
            (
                Event::Error(ErrorSample {
                    t: 3,
                    code: "HTTP_5xx".into(),
                    operation: "http:orders:POST /v1/orders".into(),
                    scenario: "order-flow".into(),
                    step: "create".into(),
                    agent: "gen-1".into(),
                    message: "status 500".into(),
                    request: Some("POST /v1/orders".into()),
                    response: None,
                }),
                r#""type":"error""#,
            ),
            (
                Event::Done(AgentSummary {
                    status: RunStatus::Completed,
                    ops: vec![OpSummary {
                        id: "op".into(),
                        n: 1,
                        rps_avg: 1.0,
                        rps_peak: 1.0,
                        err: BTreeMap::new(),
                        err_pct: 0.0,
                        dropped: 0,
                        p50: 1.0,
                        p90: 1.0,
                        p95: 1.0,
                        p99: 1.0,
                        max: 1.0,
                        target: Some(2.0),
                        achieved_pct: Some(50.0),
                    }],
                    scenarios: vec![],
                    seconds: 1,
                }),
                r#""type":"done""#,
            ),
            (Event::Gap, r#""type":"gap""#),
        ];
        for (event, tag) in cases {
            let text = serde_json::to_string(&event).unwrap();
            assert!(text.contains(tag), "{text}");
            let again: Event = serde_json::from_str(&text).unwrap();
            assert_eq!(again, event);
        }
        let text = serde_json::to_string(&Event::Tick(tick())).unwrap();
        assert!(text.contains("\"bytesIn\":120400"), "{text}");
        assert!(text.contains("\"status\":\"completed\"") || !text.contains("status"));
    }

    #[test]
    fn api_error_and_info_accept_missing_optional_fields() {
        let e: ApiError =
            serde_json::from_str(r#"{"code":"AGENT_BUSY","message":"busy"}"#).unwrap();
        assert!(e.names.is_empty() && e.agent.is_none() && e.controller.is_none());
        let i: AgentInfo =
            serde_json::from_str(r#"{"version":"0.2.0","os":"linux","arch":"x86_64","cpus":8}"#)
                .unwrap();
        assert_eq!(i.busy, None);
        let item = RunListItem {
            run_id: "r".into(),
            status: RunStatus::Incomplete,
            started_at: "2026-11-02T14:30:00Z".into(),
        };
        let text = serde_json::to_string(&item).unwrap();
        assert_eq!(
            text,
            r#"{"runId":"r","status":"incomplete","startedAt":"2026-11-02T14:30:00Z"}"#
        );
    }
}
