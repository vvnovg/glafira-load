//! Частоты сценариев и покрытие профиля сьютом (спека L2 §3.2, Global Constraints).

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::profile::Computed;
use crate::registry::OpKind;
use crate::suite::schema::Suite;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoverageRow {
    pub id: String,
    pub kind: OpKind,
    /// Цель из профиля; 0, если у операции нет цифр ПРОМ.
    pub target: f64,
    /// Σ по сценариям: частота × число шагов с операцией.
    pub planned: f64,
    /// `planned - target`.
    pub delta: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Coverage {
    /// Операции профиля в его порядке, затем операции шагов без цели по алфавиту.
    pub rows: Vec<CoverageRow>,
    /// Операции `active` с цифрами (есть в `Computed.targets`), у которых нет шагов
    /// ни в одном сценарии сьюта (спека L2 §3.2). Недобор частоты виден в `delta`.
    pub uncovered: Vec<String>,
}

fn target_of(computed: &Computed, id: &str) -> Option<f64> {
    computed
        .targets
        .iter()
        .find(|t| t.id == id)
        .map(|t| t.target)
}

/// Итераций в секунду при 100 % таймлайна: цель ведущей операции ÷ число шагов с ней в сценарии.
/// Сценарий без шагов с ведущей операцией или без цели даёт 0.
pub fn scenario_rates(suite: &Suite, computed: &Computed) -> BTreeMap<String, f64> {
    suite
        .scenarios
        .iter()
        .map(|sc| {
            let drives = sc.drives();
            let steps = sc
                .steps
                .iter()
                .filter(|s| s.action.operation() == drives)
                .count();
            let rate = match target_of(computed, drives) {
                Some(target) if steps > 0 => target / steps as f64,
                _ => 0.0,
            };
            (sc.id.clone(), rate)
        })
        .collect()
}

/// Покрытие профиля сьютом: строки по операциям и список `uncovered` — операции с целью
/// (есть в `Computed.targets`), которых нет ни в одном шаге ни одного сценария.
pub fn coverage(suite: &Suite, computed: &Computed) -> Coverage {
    let rates = scenario_rates(suite, computed);
    let mut planned: BTreeMap<&str, (OpKind, f64)> = BTreeMap::new();
    // Операции, встречающиеся хотя бы в одном шаге любого сценария (независимо от частоты).
    let mut with_steps: BTreeSet<&str> = BTreeSet::new();
    for sc in &suite.scenarios {
        let rate = rates.get(&sc.id).copied().unwrap_or(0.0);
        for step in &sc.steps {
            with_steps.insert(step.action.operation());
            let e = planned
                .entry(step.action.operation())
                .or_insert((step.action.kind(), 0.0));
            e.1 += rate;
        }
    }
    let mut rows = Vec::new();
    let mut uncovered = Vec::new();
    for t in &computed.targets {
        let p = planned.remove(t.id.as_str()).map(|(_, p)| p).unwrap_or(0.0);
        if !with_steps.contains(t.id.as_str()) {
            uncovered.push(t.id.clone());
        }
        rows.push(CoverageRow {
            id: t.id.clone(),
            kind: t.kind,
            target: t.target,
            planned: p,
            delta: p - t.target,
        });
    }
    for (id, (kind, p)) in planned {
        rows.push(CoverageRow {
            id: id.into(),
            kind,
            target: 0.0,
            planned: p,
            delta: p,
        });
    }
    Coverage { rows, uncovered }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::prod_load::ProdLoad;
    use crate::profile::{ProfileFile, compute};
    use crate::registry::Registry;
    use crate::suite::parse;
    use crate::suite::schema::{Action, HttpStep, Scenario, Step};

    pub(crate) fn fixture(rel: &str) -> String {
        std::fs::read_to_string(
            concat!(env!("CARGO_MANIFEST_DIR"), "/../../dev/fixtures/suites/").to_string() + rel,
        )
        .unwrap()
    }

    pub(crate) fn computed(profile: &str) -> Computed {
        let registry: Registry =
            crate::from_yaml(&fixture("project/registry/operations.yaml")).unwrap();
        let prod: ProdLoad = crate::from_yaml(&fixture("project/load/prod-load.yaml")).unwrap();
        let file: ProfileFile = crate::from_yaml(&fixture("project/load/profile.yaml")).unwrap();
        compute(&file, profile, &registry, &prod).unwrap()
    }

    #[test]
    fn sample_suite_covers_the_nominal_profile_as_expected() {
        let suite = parse(&fixture("orders/suite.yaml")).unwrap();
        let c = computed("nominal");
        assert_eq!(
            scenario_rates(&suite, &c),
            BTreeMap::from([("order-flow".to_string(), 100.0)])
        );
        let cov = coverage(&suite, &c);
        let rows: Vec<(&str, f64, f64, f64)> = cov
            .rows
            .iter()
            .map(|r| (r.id.as_str(), r.target, r.planned, r.delta))
            .collect();
        assert_eq!(
            rows,
            vec![
                ("http:orders:GET /v1/orders", 50.0, 0.0, -50.0),
                ("http:orders:GET /v1/orders/{id}", 400.0, 100.0, -300.0),
                ("http:orders:POST /v1/orders", 100.0, 100.0, 0.0),
                ("kafka_consume:orders.created", 100.0, 100.0, 0.0),
                ("kafka_produce:orders.paid", 80.0, 100.0, 20.0),
            ]
        );
        assert_eq!(cov.uncovered, vec!["http:orders:GET /v1/orders"]);
        assert_eq!(cov.rows[0].kind, OpKind::Http);
        let peak = coverage(&suite, &computed("peak"));
        assert_eq!(peak.rows[2].planned, 300.0);
    }

    #[test]
    fn rate_divides_by_the_number_of_driving_steps_and_unknown_targets_give_zero() {
        let mut suite = parse(&fixture("orders/suite.yaml")).unwrap();
        let post = suite.scenarios[0].steps[0].clone();
        suite.scenarios[0].steps.push(post);
        let c = computed("nominal");
        assert_eq!(scenario_rates(&suite, &c)["order-flow"], 50.0);
        suite.scenarios.push(Scenario {
            id: "health".into(),
            drives: None,
            on_fail: Default::default(),
            steps: vec![Step {
                id: None,
                action: Action::Http(HttpStep {
                    operation: "http:orders:GET /v1/health".into(),
                    path: Default::default(),
                    query: Default::default(),
                    headers: Default::default(),
                    body: None,
                    body_text: None,
                    timeout_ms: None,
                }),
                expect: Default::default(),
                extract: Default::default(),
            }],
        });
        let rates = scenario_rates(&suite, &c);
        assert_eq!(rates["health"], 0.0, "операция без цифр ПРОМ");
        let cov = coverage(&suite, &c);
        let health = cov
            .rows
            .iter()
            .find(|r| r.id == "http:orders:GET /v1/health")
            .unwrap();
        assert_eq!((health.target, health.planned), (0.0, 0.0));
        assert_eq!(cov.rows.len(), 6);
    }

    /// Операция с целью, у которой есть шаг в сценарии с нулевой частотой (ведущая операция без
    /// цели), не попадает в `uncovered`: planned == 0, но шаги в сьюте есть.
    #[test]
    fn operation_with_steps_in_zero_rate_scenario_is_not_uncovered() {
        let mut suite = parse(&fixture("orders/suite.yaml")).unwrap();
        let http = |op: &str| Step {
            id: None,
            action: Action::Http(HttpStep {
                operation: op.into(),
                path: Default::default(),
                query: Default::default(),
                headers: Default::default(),
                body: None,
                body_text: None,
                timeout_ms: None,
            }),
            expect: Default::default(),
            extract: Default::default(),
        };
        suite.scenarios = vec![Scenario {
            id: "health-then-order".into(),
            drives: None,
            on_fail: Default::default(),
            steps: vec![
                http("http:orders:GET /v1/health"),
                http("http:orders:POST /v1/orders"),
            ],
        }];
        let c = computed("nominal");
        assert_eq!(scenario_rates(&suite, &c)["health-then-order"], 0.0);
        let cov = coverage(&suite, &c);
        let post = cov
            .rows
            .iter()
            .find(|r| r.id == "http:orders:POST /v1/orders")
            .unwrap();
        assert_eq!(post.planned, 0.0);
        assert!(post.target > 0.0);
        assert!(!cov.uncovered.contains(&post.id), "шаг в сьюте есть");
        assert!(
            cov.uncovered
                .contains(&"http:orders:GET /v1/orders".to_string()),
            "операция с целью и без шагов остаётся в uncovered"
        );
    }

    /// Свойство: Σ planned по операциям = Σ по сценариям (частота × число шагов).
    #[test]
    fn planned_sum_equals_rate_times_steps_over_scenarios() {
        let c = computed("peak");
        let ops: Vec<&str> = c
            .targets
            .iter()
            .map(|t| t.id.as_str())
            .chain(["http:orders:GET /v1/health"])
            .collect();
        let mut seed = 0x2545_F491_4F6C_DD1Du64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let http = |op: &str| {
            Action::Http(HttpStep {
                operation: op.into(),
                path: Default::default(),
                query: Default::default(),
                headers: Default::default(),
                body: None,
                body_text: None,
                timeout_ms: None,
            })
        };
        for round in 0..50 {
            let mut suite = parse(&fixture("orders/suite.yaml")).unwrap();
            suite.scenarios.clear();
            for s in 0..(1 + next() % 4) {
                let n = 1 + next() % 5;
                let steps: Vec<Step> = (0..n)
                    .map(|_| Step {
                        id: None,
                        action: http(ops[(next() % ops.len() as u64) as usize]),
                        expect: Default::default(),
                        extract: Default::default(),
                    })
                    .collect();
                suite.scenarios.push(Scenario {
                    id: format!("s{round}-{s}"),
                    drives: None,
                    on_fail: Default::default(),
                    steps,
                });
            }
            let rates = scenario_rates(&suite, &c);
            let by_scenarios: f64 = suite
                .scenarios
                .iter()
                .map(|sc| rates[&sc.id] * sc.steps.len() as f64)
                .sum();
            let by_ops: f64 = coverage(&suite, &c).rows.iter().map(|r| r.planned).sum();
            assert!(
                (by_ops - by_scenarios).abs() < 1e-6,
                "round {round}: {by_ops} != {by_scenarios}"
            );
        }
    }
}
