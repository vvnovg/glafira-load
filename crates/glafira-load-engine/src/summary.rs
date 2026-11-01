//! Сводка прогона и отчёт смока (спека L2 §6.1–§6.2).

use glafira_load_model::suite::{OnFail, Suite};
use serde::{Deserialize, Serialize};

use crate::agent_api::{AgentSummary, ErrorSample};

/// Код шага, который не выполнялся из-за ошибки предыдущего при `on_fail: stop`.
pub const SKIPPED: &str = "SKIPPED";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SmokeStep {
    pub id: String,
    /// Id операции шага — отчёт читается без сьюта (вкладка «Прогон», Task 16).
    pub operation: String,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SmokeScenario {
    pub id: String,
    pub steps: Vec<SmokeStep>,
}

/// `smoke.yaml`: сценарий → шаг → `ok` либо код и причина.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct SmokeReport {
    pub scenarios: Vec<SmokeScenario>,
}

impl SmokeReport {
    /// Шаги сьюта в порядке объявления; ошибка шага — первый пример с его сценарием и id.
    pub fn from_errors(suite: &Suite, errors: &[ErrorSample]) -> Self {
        let scenarios = suite
            .scenarios
            .iter()
            .map(|sc| {
                let mut failed = false;
                let steps = sc
                    .step_ids()
                    .into_iter()
                    .zip(sc.steps.iter())
                    .map(|(id, step)| {
                        let operation = step.action.operation().to_string();
                        if failed && sc.on_fail == OnFail::Stop {
                            return SmokeStep {
                                id,
                                operation,
                                ok: false,
                                code: Some(SKIPPED.into()),
                                message: Some("previous step failed".into()),
                            };
                        }
                        match errors.iter().find(|e| e.scenario == sc.id && e.step == id) {
                            Some(e) => {
                                failed = true;
                                SmokeStep {
                                    id,
                                    operation,
                                    ok: false,
                                    code: Some(e.code.clone()),
                                    message: Some(e.message.clone()),
                                }
                            }
                            None => SmokeStep {
                                id,
                                operation,
                                ok: true,
                                code: None,
                                message: None,
                            },
                        }
                    })
                    .collect();
                SmokeScenario {
                    id: sc.id.clone(),
                    steps,
                }
            })
            .collect();
        Self { scenarios }
    }

    pub fn failed_steps(&self) -> usize {
        self.scenarios
            .iter()
            .flat_map(|s| s.steps.iter())
            .filter(|s| !s.ok)
            .count()
    }

    pub fn is_ok(&self) -> bool {
        self.failed_steps() == 0
    }
}

/// Копия сводки с операциями и сценариями по id: `summary.yaml` стабилен.
pub fn sorted_summary(s: &AgentSummary) -> AgentSummary {
    let mut out = s.clone();
    out.ops.sort_by(|a, b| a.id.cmp(&b.id));
    out.scenarios.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SUITE: &str = r#"
format: 1
name: smoke-test
scenarios:
  - id: flow
    steps:
      - id: create
        http: { operation: "http:orders:POST /v1/orders" }
      - http: { operation: "http:orders:GET /v1/orders/{id}" }
      - id: pay
        kafka_produce: { operation: "kafka_produce:orders.paid" }
  - id: lenient
    on_fail: continue
    steps:
      - http: { operation: "http:orders:GET /v1/orders/{id}" }
      - http: { operation: "http:orders:GET /v1/orders/{id}" }
"#;

    fn error(scenario: &str, step: &str, code: &str) -> ErrorSample {
        ErrorSample {
            t: 0,
            code: code.into(),
            operation: "op".into(),
            scenario: scenario.into(),
            step: step.into(),
            agent: "local".into(),
            message: format!("{code} happened"),
            request: None,
            response: None,
        }
    }

    #[test]
    fn report_marks_failed_skipped_and_ok_steps() {
        let suite = glafira_load_model::suite::parse(SUITE).unwrap();
        let report = SmokeReport::from_errors(
            &suite,
            &[
                error("flow", "step2", "EXPECT_STATUS"),
                error("lenient", "step1", "TIMEOUT"),
            ],
        );
        let flow = &report.scenarios[0];
        assert_eq!(
            flow.steps.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
            ["create", "step2", "pay"]
        );
        assert_eq!(
            flow.steps
                .iter()
                .map(|s| s.operation.as_str())
                .collect::<Vec<_>>(),
            [
                "http:orders:POST /v1/orders",
                "http:orders:GET /v1/orders/{id}",
                "kafka_produce:orders.paid"
            ]
        );
        assert!(flow.steps[0].ok && flow.steps[0].code.is_none());
        assert_eq!(
            (flow.steps[1].ok, flow.steps[1].code.as_deref()),
            (false, Some("EXPECT_STATUS"))
        );
        assert_eq!(
            flow.steps[1].message.as_deref(),
            Some("EXPECT_STATUS happened")
        );
        assert_eq!(
            (flow.steps[2].ok, flow.steps[2].code.as_deref()),
            (false, Some(SKIPPED)),
            "после ошибки при on_fail: stop"
        );
        let lenient = &report.scenarios[1];
        assert!(
            !lenient.steps[0].ok && lenient.steps[1].ok,
            "on_fail: continue не пропускает шаги"
        );
        assert_eq!(report.failed_steps(), 3);
        assert!(!report.is_ok());
        assert!(SmokeReport::from_errors(&suite, &[]).is_ok());

        let text = glafira_load_model::to_yaml(&report);
        assert!(
            text.contains("- id: create\n    operation: ") && text.contains("\n    ok: true\n"),
            "{text}"
        );
        assert!(
            !text.contains("message: null"),
            "пустые поля не пишутся: {text}"
        );
        let back: SmokeReport = glafira_load_model::from_yaml(&text).unwrap();
        assert_eq!(back, report);
    }
}
