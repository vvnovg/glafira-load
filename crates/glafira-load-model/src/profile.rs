//! Профиль нагрузки `load/profile.yaml` и его расчёт (спека L1 §8).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::prod_load::ProdLoad;
use crate::registry::{OpKind, Registry, Status};

pub const PROFILE_FILE: &str = "load/profile.yaml";
pub const FORMAT: u32 = 1;

fn one() -> f64 {
    1.0
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Override {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub k_env: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub k_growth: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Overrides {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub services: BTreeMap<String, Override>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub operations: BTreeMap<String, Override>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Coefficients {
    /// Мощность стенда относительно ПРОМ.
    #[serde(default = "one")]
    pub k_env: f64,
    /// Запас роста.
    #[serde(default = "one")]
    pub k_growth: f64,
    #[serde(default)]
    pub overrides: Overrides,
}

impl Default for Coefficients {
    fn default() -> Self {
        Self {
            k_env: 1.0,
            k_growth: 1.0,
            overrides: Overrides::default(),
        }
    }
}

impl Coefficients {
    /// Коэффициенты операции: переопределение операции, иначе сервиса, иначе общие.
    pub fn for_operation(&self, service: &str, id: &str) -> (f64, f64) {
        let op = self.overrides.operations.get(id);
        let svc = self.overrides.services.get(service);
        let pick = |f: fn(&Override) -> Option<f64>, base: f64| {
            op.and_then(f).or_else(|| svc.and_then(f)).unwrap_or(base)
        };
        (
            pick(|o| o.k_env, self.k_env),
            pick(|o| o.k_growth, self.k_growth),
        )
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Basis {
    Avg,
    Peak,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Phase {
    /// Линейно от текущего уровня к `to_pct` за `minutes`.
    Ramp {
        minutes: f64,
        to_pct: f64,
    },
    Steady {
        minutes: f64,
    },
    /// Ступени `step_pct`, 2·`step_pct`, … до `max_pct` включительно, каждая по `step_minutes`.
    Steps {
        step_pct: f64,
        step_minutes: f64,
        max_pct: f64,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Profile {
    pub basis: Basis,
    #[serde(default)]
    pub phases: Vec<Phase>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProfileFile {
    pub format: u32,
    #[serde(default)]
    pub coefficients: Coefficients,
    #[serde(default)]
    pub profiles: BTreeMap<String, Profile>,
}

impl Default for ProfileFile {
    /// Три профиля из спеки §8.1: nominal, peak, stress.
    fn default() -> Self {
        let mut profiles = BTreeMap::new();
        profiles.insert(
            "nominal".into(),
            Profile {
                basis: Basis::Avg,
                phases: vec![
                    Phase::Ramp {
                        minutes: 10.0,
                        to_pct: 100.0,
                    },
                    Phase::Steady { minutes: 60.0 },
                    Phase::Ramp {
                        minutes: 5.0,
                        to_pct: 0.0,
                    },
                ],
            },
        );
        profiles.insert(
            "peak".into(),
            Profile {
                basis: Basis::Peak,
                phases: vec![
                    Phase::Ramp {
                        minutes: 5.0,
                        to_pct: 100.0,
                    },
                    Phase::Steady { minutes: 30.0 },
                ],
            },
        );
        profiles.insert(
            "stress".into(),
            Profile {
                basis: Basis::Peak,
                phases: vec![Phase::Steps {
                    step_pct: 25.0,
                    step_minutes: 10.0,
                    max_pct: 300.0,
                }],
            },
        );
        Self {
            format: FORMAT,
            coefficients: Coefficients::default(),
            profiles,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Target {
    pub id: String,
    pub service: String,
    pub kind: OpKind,
    pub basis_value: f64,
    pub k_env: f64,
    pub k_growth: f64,
    pub target: f64,
    pub share: f64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct Point {
    pub minute: f64,
    pub pct: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Computed {
    pub profile: String,
    pub basis: Basis,
    pub targets: Vec<Target>,
    pub total: f64,
    /// Активные операции без цифр ПРОМ.
    pub uncovered: Vec<String>,
    pub timeline: Vec<Point>,
    pub total_minutes: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProfileError {
    #[error("profile not found: {0}")]
    UnknownProfile(String),
}

/// Ряд «минута → % цели». Начинается с (0, 0); у ступеней скачок — две точки с одной минутой.
pub fn timeline(phases: &[Phase]) -> Vec<Point> {
    let mut out = vec![Point {
        minute: 0.0,
        pct: 0.0,
    }];
    let (mut t, mut pct) = (0.0, 0.0);
    for phase in phases {
        match *phase {
            Phase::Ramp { minutes, to_pct } => {
                t += minutes;
                pct = to_pct;
                out.push(Point { minute: t, pct });
            }
            Phase::Steady { minutes } => {
                t += minutes;
                out.push(Point { minute: t, pct });
            }
            Phase::Steps {
                step_pct,
                step_minutes,
                max_pct,
            } => {
                if step_pct <= 0.0 || step_minutes <= 0.0 {
                    continue;
                }
                let mut level = step_pct;
                while level <= max_pct + 1e-9 {
                    out.push(Point {
                        minute: t,
                        pct: level,
                    });
                    t += step_minutes;
                    out.push(Point {
                        minute: t,
                        pct: level,
                    });
                    level += step_pct;
                }
                pct = out.last().map(|p| p.pct).unwrap_or(pct);
            }
        }
    }
    out
}

pub fn compute(
    file: &ProfileFile,
    name: &str,
    registry: &Registry,
    prod: &ProdLoad,
) -> Result<Computed, ProfileError> {
    let profile = file
        .profiles
        .get(name)
        .ok_or_else(|| ProfileError::UnknownProfile(name.into()))?;
    let mut targets = Vec::new();
    let mut uncovered = Vec::new();
    for op in registry
        .operations
        .iter()
        .filter(|o| o.status == Status::Active)
    {
        let Some(f) = prod.operations.get(&op.draft.id) else {
            uncovered.push(op.draft.id.clone());
            continue;
        };
        let basis_value = match profile.basis {
            Basis::Avg => f.avg,
            Basis::Peak => f.peak,
        };
        let (k_env, k_growth) = file
            .coefficients
            .for_operation(&op.draft.service, &op.draft.id);
        targets.push(Target {
            id: op.draft.id.clone(),
            service: op.draft.service.clone(),
            kind: op.draft.kind,
            basis_value,
            k_env,
            k_growth,
            target: basis_value * k_env * k_growth,
            share: 0.0,
        });
    }
    let total: f64 = targets.iter().map(|t| t.target).sum();
    if total > 0.0 {
        for t in &mut targets {
            t.share = t.target / total;
        }
    }
    let timeline = timeline(&profile.phases);
    let total_minutes = timeline.last().map(|p| p.minute).unwrap_or(0.0);
    Ok(Computed {
        profile: name.into(),
        basis: profile.basis,
        targets,
        total,
        uncovered,
        timeline,
        total_minutes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::merge::merge;
    use crate::prod_load::Figures;
    use crate::registry::{Address, OperationDraft, Seen, Source};

    fn registry() -> Registry {
        let mut r = Registry::default();
        let seen = Seen::default();
        let mk = |svc: &str, path: &str| {
            OperationDraft::new(
                svc,
                OpKind::Http,
                Address::Http {
                    method: "GET".into(),
                    path: path.into(),
                },
                Source::manual(),
            )
        };
        merge(
            &mut r,
            "orders",
            vec![
                mk("orders", "/a"),
                mk("orders", "/b"),
                mk("orders", "/none"),
            ],
            &seen,
        );
        merge(&mut r, "billing", vec![mk("billing", "/pay")], &seen);
        r
    }

    fn prod() -> ProdLoad {
        let mut p = ProdLoad::default();
        for (id, avg, peak) in [
            ("http:orders:GET /a", 100.0, 300.0),
            ("http:orders:GET /b", 50.0, 60.0),
            ("http:billing:GET /pay", 50.0, 100.0),
        ] {
            p.operations.insert(
                id.into(),
                Figures {
                    avg,
                    peak,
                    ..Default::default()
                },
            );
        }
        p
    }

    #[test]
    fn targets_use_overrides_and_shares_sum_to_one() {
        let mut file = ProfileFile::default();
        file.coefficients.k_env = 0.5;
        file.coefficients.k_growth = 2.0;
        file.coefficients.overrides.services.insert(
            "billing".into(),
            Override {
                k_env: Some(0.1),
                k_growth: None,
            },
        );
        file.coefficients.overrides.operations.insert(
            "http:orders:GET /b".into(),
            Override {
                k_env: None,
                k_growth: Some(4.0),
            },
        );
        let c = compute(&file, "nominal", &registry(), &prod()).unwrap();
        let by: BTreeMap<&str, &Target> = c.targets.iter().map(|t| (t.id.as_str(), t)).collect();
        assert_eq!(by["http:orders:GET /a"].target, 100.0);
        assert_eq!(by["http:orders:GET /b"].target, 50.0 * 0.5 * 4.0);
        assert_eq!(by["http:billing:GET /pay"].target, 50.0 * 0.1 * 2.0);
        assert!((c.targets.iter().map(|t| t.share).sum::<f64>() - 1.0).abs() < 1e-9);
        assert_eq!(c.uncovered, vec!["http:orders:GET /none"]);
        assert_eq!(c.total_minutes, 75.0);
        let peak = compute(&file, "peak", &registry(), &prod()).unwrap();
        assert_eq!(
            peak.targets
                .iter()
                .find(|t| t.id == "http:orders:GET /a")
                .unwrap()
                .basis_value,
            300.0
        );
    }

    #[test]
    fn raising_any_coefficient_never_lowers_a_target() {
        let base = compute(&ProfileFile::default(), "nominal", &registry(), &prod()).unwrap();
        for k in [1.5, 3.0] {
            let mut f = ProfileFile::default();
            f.coefficients.k_growth = k;
            let c = compute(&f, "nominal", &registry(), &prod()).unwrap();
            for (a, b) in base.targets.iter().zip(&c.targets) {
                assert!(
                    b.target >= a.target,
                    "{}: {} < {}",
                    a.id,
                    b.target,
                    a.target
                );
            }
        }
    }

    #[test]
    fn timeline_is_continuous_at_phase_boundaries() {
        let pts = timeline(&[
            Phase::Ramp {
                minutes: 10.0,
                to_pct: 100.0,
            },
            Phase::Steady { minutes: 20.0 },
            Phase::Steps {
                step_pct: 50.0,
                step_minutes: 5.0,
                max_pct: 150.0,
            },
            Phase::Ramp {
                minutes: 5.0,
                to_pct: 0.0,
            },
        ]);
        let m: Vec<(f64, f64)> = pts.iter().map(|p| (p.minute, p.pct)).collect();
        assert_eq!(
            m,
            vec![
                (0.0, 0.0),
                (10.0, 100.0),
                (30.0, 100.0),
                (30.0, 50.0),
                (35.0, 50.0),
                (35.0, 100.0),
                (40.0, 100.0),
                (40.0, 150.0),
                (45.0, 150.0),
                (50.0, 0.0)
            ]
        );
        assert!(pts.windows(2).all(|w| w[1].minute >= w[0].minute));
        assert_eq!(
            timeline(&[]),
            vec![Point {
                minute: 0.0,
                pct: 0.0
            }]
        );
    }

    #[test]
    fn unknown_profile_and_empty_prod() {
        assert_eq!(
            compute(&ProfileFile::default(), "nope", &registry(), &prod()).unwrap_err(),
            ProfileError::UnknownProfile("nope".into())
        );
        let c = compute(
            &ProfileFile::default(),
            "nominal",
            &registry(),
            &ProdLoad::default(),
        )
        .unwrap();
        assert_eq!((c.targets.len(), c.uncovered.len(), c.total), (0, 4, 0.0));
    }

    #[test]
    fn default_file_round_trips() {
        let f = ProfileFile::default();
        let text = crate::to_yaml(&f);
        assert!(text.contains("kind: steps"), "{text}");
        assert_eq!(crate::from_yaml::<ProfileFile>(&text).unwrap(), f);
    }
}
