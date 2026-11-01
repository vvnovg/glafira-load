//! Бандлы прогона: доли целей и строк данных по весам агентов, `RunRequest` каждого агента (спека §4.4–§4.5).

use std::collections::BTreeMap;

use glafira_load_model::project::Environment;
use glafira_load_model::suite::DataMode;

use crate::agent_api::{EnvAddresses, RunRequest};
use crate::context::{DataFormat, is_inside_project};
use crate::controller::{CONTROLLER_VERSION, EngineError, RunPlan};

/// Цели операций, умноженные на долю каждого агента (`weight / Σweight`).
pub fn split_targets(
    targets: &BTreeMap<String, f64>,
    agents: &[(String, u32)],
) -> Vec<BTreeMap<String, f64>> {
    let total: u32 = agents.iter().map(|(_, w)| *w).sum();
    agents
        .iter()
        .map(|(_, w)| {
            let share = if total == 0 {
                0.0
            } else {
                f64::from(*w) / f64::from(total)
            };
            targets
                .iter()
                .map(|(id, v)| (id.clone(), v * share))
                .collect()
        })
        .collect()
}

/// Строки `slice`-набора по агентам: строка `i` уходит агенту, которому принадлежит позиция
/// `i mod Σweight` круга, где агент с весом `w` занимает `w` позиций подряд (отступление плана 4).
pub fn slice_rows(rows: &[String], agents: &[(String, u32)]) -> Vec<Vec<String>> {
    let mut out: Vec<Vec<String>> = vec![Vec::new(); agents.len()];
    let mut owner = Vec::new();
    for (i, (_, w)) in agents.iter().enumerate() {
        owner.extend(std::iter::repeat_n(i, *w as usize));
    }
    if owner.is_empty() {
        return out;
    }
    for (i, row) in rows.iter().enumerate() {
        out[owner[i % owner.len()]].push(row.clone());
    }
    out
}

/// Цели операций плана: `id → RPS`.
pub fn targets_of(plan: &RunPlan) -> BTreeMap<String, f64> {
    crate::local::targets_of(&plan.computed)
}

/// Адреса стенда для `RunRequest`.
pub fn env_addresses(env: &Environment) -> EnvAddresses {
    EnvAddresses {
        http: env.http.clone(),
        kafka: env.kafka.clone(),
        schema_registry: env.schema_registry.clone(),
    }
}

/// Потолок одновременности агента: ключ запуска, иначе `defaults.max_in_flight` сьюта.
pub fn max_in_flight(plan: &RunPlan) -> u32 {
    plan.max_in_flight
        .unwrap_or(plan.suite.defaults.max_in_flight)
}

/// Бандл одного агента: запрос и файлы данных (ключ — имя набора из `suite.data`;
/// агент разбирает содержимое по расширению `suite.data[name].path`).
#[derive(Debug, Clone, PartialEq)]
pub struct Bundle {
    pub request: RunRequest,
    pub data: BTreeMap<String, Vec<u8>>,
}

/// Записи CSV без заголовка: каждая — готовая строка файла с переводом строки в конце
/// (поле в кавычках с переводом строки остаётся одной записью).
fn csv_parts(name: &str, text: &str) -> Result<(String, Vec<String>), EngineError> {
    let bad = |e: csv::Error| EngineError::new("INVALID_INPUT", format!("data `{name}`: {e}"));
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(true)
        .from_reader(text.as_bytes());
    let header = encode_csv(reader.headers().map_err(bad)?);
    let mut rows = Vec::new();
    for rec in reader.records() {
        rows.push(encode_csv(&rec.map_err(bad)?));
    }
    Ok((header, rows))
}

fn encode_csv(rec: &csv::StringRecord) -> String {
    let mut w = csv::Writer::from_writer(Vec::new());
    w.write_record(rec)
        .expect("writing a record into memory does not fail");
    let bytes = w.into_inner().expect("flushing into memory does not fail");
    String::from_utf8(bytes).expect("csv of UTF-8 fields is UTF-8")
}

/// Бандлы всех агентов: цели по весам, `slice`-наборы нарезаны по записям (CSV — с заголовком
/// у каждого агента, JSON Lines — по непустым строкам), `shared` — целиком, байт в байт.
pub fn bundles(
    plan: &RunPlan,
    run_id: &str,
    agents: &[(String, u32)],
) -> Result<Vec<Bundle>, EngineError> {
    let shares = split_targets(&targets_of(plan), agents);
    let mut data: Vec<BTreeMap<String, Vec<u8>>> = vec![BTreeMap::new(); agents.len()];
    for (name, set) in &plan.suite.data {
        if !is_inside_project(&set.path) {
            return Err(EngineError::new(
                "PATH_OUTSIDE_PROJECT",
                format!("data `{name}`: path `{}` is outside the project", set.path),
            ));
        }
        let path = plan.root.join(&set.path);
        let read_err =
            |e: std::io::Error| EngineError::io(format!("cannot read data file {}: {e}", set.path));
        match set.mode {
            DataMode::Shared => {
                let bytes = std::fs::read(&path).map_err(read_err)?;
                for d in &mut data {
                    d.insert(name.clone(), bytes.clone());
                }
            }
            DataMode::Slice => {
                let text = std::fs::read_to_string(&path).map_err(read_err)?;
                let format = DataFormat::from_path(&set.path)
                    .map_err(|e| EngineError::new("INVALID_INPUT", e))?;
                let (header, rows) = match format {
                    DataFormat::Csv => csv_parts(name, &text)?,
                    DataFormat::JsonLines => (
                        String::new(),
                        text.lines()
                            .filter(|l| !l.trim().is_empty())
                            .map(|l| format!("{l}\n"))
                            .collect(),
                    ),
                };
                for (i, part) in slice_rows(&rows, agents).into_iter().enumerate() {
                    let mut out = header.clone();
                    part.iter().for_each(|row| out.push_str(row));
                    data[i].insert(name.clone(), out.into_bytes());
                }
            }
        }
    }
    Ok(agents
        .iter()
        .zip(shares)
        .zip(data)
        .map(|(((id, _), targets), data)| Bundle {
            request: RunRequest {
                run_id: run_id.to_string(),
                agent_id: id.clone(),
                suite_name: plan.suite_name.clone(),
                controller_version: CONTROLLER_VERSION.to_string(),
                suite_format: plan.suite.format,
                env: env_addresses(&plan.env),
                secrets: plan.suite.secrets.clone(),
                targets,
                timeline: plan.computed.timeline.clone(),
                total_minutes: plan.computed.total_minutes,
                max_in_flight: max_in_flight(plan),
                arrival: plan.suite.defaults.arrival,
                mode: plan.mode,
                kafka: plan.suite.defaults.kafka.clone(),
                response_schemas: plan.response_schemas.clone(),
                avro_schemas: plan.avro_schemas.clone(),
            },
            data,
        })
        .collect())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    use glafira_load_model::profile::{Basis, Computed, Point, Target};
    use glafira_load_model::project::{Agent, EnvKind, Environment};
    use glafira_load_model::registry::OpKind;
    use glafira_load_model::suite::parse;
    use std::path::PathBuf;

    use crate::agent_api::Mode;

    fn agents() -> Vec<(String, u32)> {
        vec![("a".into(), 1), ("b".into(), 2)]
    }

    #[test]
    fn split_targets_sums_back_to_the_original() {
        let mut targets = BTreeMap::new();
        targets.insert("x".to_string(), 90.0);
        targets.insert("y".to_string(), 1.0);
        let parts = split_targets(&targets, &agents());
        assert_eq!(parts.len(), 2);
        assert!((parts[0]["x"] - 30.0).abs() < 1e-9 && (parts[1]["x"] - 60.0).abs() < 1e-9);
        for id in ["x", "y"] {
            let sum: f64 = parts.iter().map(|p| p[id]).sum();
            assert!((sum - targets[id]).abs() < 1e-9, "{id}: {sum}");
        }
        assert!(split_targets(&targets, &[]).is_empty());
    }

    #[test]
    fn slice_rows_follows_weights_round_robin() {
        let rows: Vec<String> = (0..100).map(|i| format!("r{i}")).collect();
        let parts = slice_rows(&rows, &agents());
        assert!(
            (parts[0].len() as i64 - 33).abs() <= 1,
            "{}",
            parts[0].len()
        );
        assert!(
            (parts[1].len() as i64 - 67).abs() <= 1,
            "{}",
            parts[1].len()
        );
        assert_eq!(parts[0].len() + parts[1].len(), 100);
        assert_eq!(&parts[0][..2], &["r0", "r3"]);
        assert_eq!(&parts[1][..3], &["r1", "r2", "r4"]);
        assert!(slice_rows(&rows, &[]).is_empty());
    }

    pub(crate) const SUITE: &str = "format: 1
name: orders
secrets: [API_TOKEN]
data:
  users: { path: data/users.csv, mode: slice }
  skus: data/skus.jsonl
  events: { path: data/events.jsonl, mode: slice }
defaults:
  max_in_flight: 50
scenarios:
  - id: ping
    steps:
      - http: { operation: \"http:orders:GET /ping\", query: { user: \"{{users.id}}\" } }
        expect: { status: 200 }
";

    pub(crate) fn plan(root: PathBuf) -> RunPlan {
        let suite = parse(SUITE).unwrap();
        RunPlan {
            root,
            suite_name: "orders".into(),
            suite,
            suite_text: SUITE.into(),
            profile: "short".into(),
            computed: Computed {
                profile: "short".into(),
                basis: Basis::Avg,
                targets: vec![Target {
                    id: "http:orders:GET /ping".into(),
                    service: "orders".into(),
                    kind: OpKind::Http,
                    basis_value: 30.0,
                    k_env: 1.0,
                    k_growth: 1.0,
                    target: 30.0,
                    share: 1.0,
                }],
                total: 30.0,
                uncovered: vec![],
                timeline: vec![
                    Point {
                        minute: 0.0,
                        pct: 0.0,
                    },
                    Point {
                        minute: 0.1,
                        pct: 100.0,
                    },
                ],
                total_minutes: 0.1,
            },
            env: Environment {
                id: "test".into(),
                kind: EnvKind::Test,
                http: Some("http://127.0.0.1:1".into()),
                kafka: None,
                grpc: None,
                sql: None,
                schema_registry: None,
                agents: vec![
                    Agent {
                        id: "a".into(),
                        url: "http://127.0.0.1:2".into(),
                        weight: 1,
                    },
                    Agent {
                        id: "b".into(),
                        url: "http://127.0.0.1:3".into(),
                        weight: 2,
                    },
                ],
                extra: Default::default(),
            },
            mode: Mode::Run,
            max_in_flight: None,
            out_dir: None,
            response_schemas: BTreeMap::new(),
            avro_schemas: BTreeMap::new(),
        }
    }

    /// Папка проекта с наборами `users.csv` (9 строк), `skus.jsonl` и `events.jsonl` (6 строк).
    fn project() -> tempfile::TempDir {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("data")).unwrap();
        let csv: String = std::iter::once("id,name".to_string())
            .chain((0..9).map(|i| format!("{i},user{i}")))
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(tmp.path().join("data/users.csv"), csv).unwrap();
        std::fs::write(
            tmp.path().join("data/skus.jsonl"),
            "{\"sku\":\"a\"}\n{\"sku\":\"b\"}\n",
        )
        .unwrap();
        let events: String = (0..6).map(|i| format!("{{\"e\":{i}}}\n\n")).collect();
        std::fs::write(tmp.path().join("data/events.jsonl"), events).unwrap();
        tmp
    }

    #[test]
    fn bundles_slice_csv_with_header_and_share_jsonl() {
        let tmp = project();
        let mut plan = plan(tmp.path().to_path_buf());
        plan.response_schemas
            .insert("http:orders:GET /ping".into(), Default::default());
        plan.avro_schemas.insert(
            "kafka:orders:produce:x".into(),
            "{\"type\":\"string\"}".into(),
        );
        let b = bundles(&plan, "run-1", &agents()).unwrap();
        assert_eq!(b.len(), 2);
        let users_a = String::from_utf8(b[0].data["users"].clone()).unwrap();
        let users_b = String::from_utf8(b[1].data["users"].clone()).unwrap();
        assert!(users_a.starts_with("id,name\n") && users_b.starts_with("id,name\n"));
        assert_eq!(users_a.lines().count() - 1, 3);
        assert_eq!(users_b.lines().count() - 1, 6);
        assert_eq!(users_a, "id,name\n0,user0\n3,user3\n6,user6\n");
        let events_a = String::from_utf8(b[0].data["events"].clone()).unwrap();
        assert_eq!(
            events_a, "{\"e\":0}\n{\"e\":3}\n",
            "jsonl: без заголовка, пустые строки отброшены"
        );
        assert_eq!(b[0].data["skus"], b[1].data["skus"]);
        assert_eq!(
            b[0].data["skus"],
            std::fs::read(tmp.path().join("data/skus.jsonl")).unwrap(),
            "shared не режется"
        );
        let r = &b[1].request;
        assert_eq!(
            (
                r.run_id.as_str(),
                r.agent_id.as_str(),
                r.suite_name.as_str()
            ),
            ("run-1", "b", "orders")
        );
        assert!((r.targets["http:orders:GET /ping"] - 20.0).abs() < 1e-9);
        assert!((b[0].request.targets["http:orders:GET /ping"] - 10.0).abs() < 1e-9);
        assert_eq!(
            (r.max_in_flight, r.suite_format, r.secrets.as_slice()),
            (50, 1, &["API_TOKEN".to_string()][..])
        );
        assert_eq!(r.env.http.as_deref(), Some("http://127.0.0.1:1"));
        assert_eq!(r.controller_version, CONTROLLER_VERSION);
        assert_eq!((r.mode, r.total_minutes), (Mode::Run, 0.1));
        assert_eq!(r.timeline, plan.computed.timeline);
        assert_eq!(r.response_schemas, plan.response_schemas);
        assert_eq!(r.avro_schemas, plan.avro_schemas);
        let mut capped = plan.clone();
        capped.max_in_flight = Some(7);
        assert_eq!(
            bundles(&capped, "run-1", &agents()).unwrap()[0]
                .request
                .max_in_flight,
            7
        );
        std::fs::remove_file(tmp.path().join("data/skus.jsonl")).unwrap();
        assert_eq!(bundles(&plan, "run-1", &agents()).unwrap_err().code, "IO");
    }

    #[test]
    fn bundles_keep_quoted_multiline_csv_rows_whole() {
        let tmp = project();
        std::fs::write(
            tmp.path().join("data/users.csv"),
            "id,name\n1,\"first\nline\"\n2,plain\n\n3,\"a,b\"\n",
        )
        .unwrap();
        let plan = plan(tmp.path().to_path_buf());
        let b = bundles(&plan, "run-1", &agents()).unwrap();
        let read = |bytes: &[u8]| crate::context::parse_data("users.csv", bytes).unwrap();
        let a = read(&b[0].data["users"]);
        let bb = read(&b[1].data["users"]);
        assert_eq!(a.columns, vec!["id", "name"]);
        assert_eq!(bb.columns, vec!["id", "name"]);
        let cells = |t: &crate::context::DataTable| -> Vec<String> {
            t.rows
                .iter()
                .map(|r| r[1].as_str().unwrap().to_string())
                .collect()
        };
        assert_eq!(cells(&a), vec!["first\nline"]);
        assert_eq!(cells(&bb), vec!["plain", "a,b"]);
    }

    #[test]
    fn bundles_give_every_agent_the_header_even_without_rows() {
        let tmp = project();
        std::fs::write(tmp.path().join("data/users.csv"), "id,name\n1,x\n").unwrap();
        let plan = plan(tmp.path().to_path_buf());
        let b = bundles(&plan, "run-1", &agents()).unwrap();
        assert_eq!(b[0].data["users"], b"id,name\n1,x\n");
        assert_eq!(b[1].data["users"], b"id,name\n");
    }

    #[test]
    fn bundles_refuse_data_outside_the_project() {
        let tmp = project();
        let mut plan = plan(tmp.path().to_path_buf());
        plan.suite.data.get_mut("skus").unwrap().path = "../skus.jsonl".into();
        let e = bundles(&plan, "run-1", &agents()).unwrap_err();
        assert_eq!(e.code, "PATH_OUTSIDE_PROJECT", "{e:?}");
    }

    #[test]
    fn helpers_read_the_plan() {
        let tmp = tempfile::tempdir().unwrap();
        let plan = plan(tmp.path().to_path_buf());
        assert_eq!(targets_of(&plan)["http:orders:GET /ping"], 30.0);
        assert_eq!(max_in_flight(&plan), 50);
        let env = env_addresses(&plan.env);
        assert_eq!(
            (env.http.as_deref(), env.kafka),
            (Some("http://127.0.0.1:1"), None)
        );
    }
}
