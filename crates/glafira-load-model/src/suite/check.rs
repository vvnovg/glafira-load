//! Проверка сьюта без сети и диска (спека L2 §3.5): схема, операции реестра, выражения, наборы
//! данных, секреты, уникальность идентификаторов, ссылки на шаги, ведущие операции, Avro.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::project::{Environment, is_id};
use crate::registry::{OpKind, Registry, Status};
use crate::suite::FORMAT;
use crate::suite::expr::{Expr, PathSeg, parse_template, step_templates};
use crate::suite::lines::line_of;
use crate::suite::schema::{Action, Encoding, StatusExpect, Suite, parse};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    Error,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    pub level: Level,
    pub file: String,
    pub line: Option<u32>,
    pub message: String,
}

/// Что приложение узнало о файле набора: колонки (заголовок CSV или ключи первой строки JSONL) и
/// число строк.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct DataInfo {
    pub columns: Vec<String>,
    pub rows: usize,
}

/// Имена наборов, занятые формами выражений.
pub const RESERVED_SET_NAMES: [&str; 4] = ["steps", "env", "run", "agent"];

struct Report<'a> {
    text: &'a str,
    file: &'a str,
    findings: Vec<Finding>,
}

impl Report<'_> {
    fn push(&mut self, level: Level, path: &[PathSeg], message: String) {
        self.findings.push(Finding {
            level,
            file: self.file.to_string(),
            line: line_of(self.text, path),
            message,
        });
    }
    fn error(&mut self, path: &[PathSeg], message: String) {
        self.push(Level::Error, path, message);
    }
    fn warning(&mut self, path: &[PathSeg], message: String) {
        self.push(Level::Warning, path, message);
    }
}

fn key(k: &str) -> PathSeg {
    PathSeg::Key(k.into())
}

fn path(prefix: &[PathSeg], keys: &[&str]) -> Vec<PathSeg> {
    let mut p = prefix.to_vec();
    p.extend(keys.iter().map(|k| key(k)));
    p
}

/// Идентификатор шага, сценария, имя `extract` или секрета: `[A-Za-z0-9_-]{1,64}` — он попадает в
/// `steps.<id>.<name>`, где точка и пробел разделители.
pub fn is_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// Путь набора внутри проекта: относительный, без `..`, `\` и диска.
fn inside_project(p: &str) -> bool {
    !p.is_empty()
        && !p.starts_with('/')
        && !p.contains('\\')
        && !p.contains(':')
        && p.split('/').all(|seg| seg != "..")
}

/// Проверка текста сьюта. `data`: имя набора → `Some(инфо)`, если файл прочитан, `None`, если
/// файла нет; набор без записи не проверяется по колонкам. `env` — стенд для `encoding: avro`.
/// Сьют возвращается, если YAML разобран, даже когда есть ошибки по смыслу.
pub fn check(
    text: &str,
    file: &str,
    registry: &Registry,
    data: &BTreeMap<String, Option<DataInfo>>,
    env: Option<&Environment>,
) -> (Option<Suite>, Vec<Finding>) {
    let suite = match parse(text) {
        Ok(s) => s,
        Err(e) => {
            return (
                None,
                vec![Finding {
                    level: Level::Error,
                    file: file.into(),
                    line: e.line,
                    message: e.message,
                }],
            );
        }
    };
    let mut r = Report {
        text,
        file,
        findings: Vec::new(),
    };
    check_header(&suite, &mut r);
    check_data(&suite, data, &mut r);
    check_secrets(&suite, &mut r);
    let mut used_sets = BTreeSet::new();
    let mut used_secrets = BTreeSet::new();
    check_defaults(&suite, &mut used_sets, &mut used_secrets, data, &mut r);
    check_scenarios(
        &suite,
        registry,
        data,
        env,
        &mut used_sets,
        &mut used_secrets,
        &mut r,
    );
    for name in suite.data.keys() {
        if !used_sets.contains(name.as_str()) {
            r.warning(
                &[key("data"), key(name)],
                format!("data set `{name}` is not used"),
            );
        }
    }
    for (i, name) in suite.secrets.iter().enumerate() {
        if !used_secrets.contains(name.as_str()) {
            r.warning(
                &[key("secrets"), PathSeg::Index(i)],
                format!("secret `{name}` is declared but not used"),
            );
        }
    }
    (Some(suite), r.findings)
}

fn check_header(suite: &Suite, r: &mut Report) {
    if suite.format != FORMAT {
        r.error(
            &[key("format")],
            format!(
                "unsupported suite format {}, expected {FORMAT}",
                suite.format
            ),
        );
    }
    if !is_id(&suite.name) {
        r.error(
            &[key("name")],
            format!("suite name `{}` must match [a-z0-9-]{{1,64}}", suite.name),
        );
    }
    if suite.scenarios.is_empty() {
        r.error(&[key("scenarios")], "suite has no scenarios".into());
    }
    if suite.defaults.max_in_flight == 0 {
        r.error(
            &[key("defaults"), key("max_in_flight")],
            "max_in_flight must be at least 1".into(),
        );
    }
    if suite.defaults.http.timeout_ms == 0 {
        r.error(
            &[key("defaults"), key("http"), key("timeout_ms")],
            "timeout_ms must be at least 1".into(),
        );
    }
}

fn check_data(suite: &Suite, data: &BTreeMap<String, Option<DataInfo>>, r: &mut Report) {
    for (name, set) in &suite.data {
        let p = [key("data"), key(name)];
        if RESERVED_SET_NAMES.contains(&name.as_str()) {
            r.error(&p, format!("data set name `{name}` is reserved"));
        } else if !is_name(name) {
            r.error(
                &p,
                format!("data set name `{name}` must match [A-Za-z0-9_-]{{1,64}}"),
            );
        }
        if !inside_project(&set.path) {
            r.error(
                &p,
                format!(
                    "data set `{name}`: path `{}` must be relative and inside the project",
                    set.path
                ),
            );
        }
        if let Some(None) = data.get(name) {
            r.error(
                &p,
                format!("data set `{name}`: file `{}` not found", set.path),
            );
        }
    }
}

fn check_secrets(suite: &Suite, r: &mut Report) {
    let mut seen = BTreeSet::new();
    for (i, name) in suite.secrets.iter().enumerate() {
        let p = [key("secrets"), PathSeg::Index(i)];
        if !is_name(name) {
            r.error(
                &p,
                format!("secret name `{name}` must match [A-Za-z0-9_-]{{1,64}}"),
            );
        } else if !seen.insert(name.as_str()) {
            r.error(&p, format!("secret `{name}` is declared twice"));
        }
    }
}

/// Проверка одного выражения: наборы и колонки, секреты, ссылки на шаги.
struct ExprScope<'a> {
    suite: &'a Suite,
    data: &'a BTreeMap<String, Option<DataInfo>>,
    /// Предыдущие шаги сценария: id → имена `extract`; `None` вне сценария (defaults).
    previous: Option<&'a BTreeMap<String, BTreeSet<String>>>,
    /// Все шаги сценария, чтобы отличить будущий шаг от неизвестного.
    all_steps: &'a [String],
}

fn check_expr(
    e: &Expr,
    scope: &ExprScope,
    p: &[PathSeg],
    used_sets: &mut BTreeSet<String>,
    used_secrets: &mut BTreeSet<String>,
    r: &mut Report,
) {
    match e {
        Expr::Data { set, col } | Expr::Pick { set, col } => {
            if !scope.suite.data.contains_key(set) {
                r.error(p, format!("unknown data set `{set}`"));
                return;
            }
            used_sets.insert(set.clone());
            if let Some(Some(info)) = scope.data.get(set)
                && !info.columns.iter().any(|c| c == col)
            {
                r.error(p, format!("data set `{set}` has no column `{col}`"));
            }
        }
        Expr::Step { step, name } => match scope.previous {
            None => r.error(p, "`steps.*` is not available in defaults".into()),
            Some(previous) => match previous.get(step) {
                Some(names) if names.contains(name) => {}
                Some(_) => r.error(p, format!("step `{step}` has no extract `{name}`")),
                None if scope.all_steps.iter().any(|s| s == step) => r.error(
                    p,
                    format!("step `{step}` runs later: only previous steps can be referenced"),
                ),
                None => r.error(p, format!("unknown step `{step}`")),
            },
        },
        Expr::Secret(name) => {
            if scope.suite.secrets.iter().any(|s| s == name) {
                used_secrets.insert(name.clone());
            } else {
                r.error(p, format!("secret `{name}` is not declared in `secrets`"));
            }
        }
        Expr::Uuid
        | Expr::Int(..)
        | Expr::Float(..)
        | Expr::Now(_)
        | Expr::Str(_)
        | Expr::Env(_)
        | Expr::RunId
        | Expr::AgentId => {}
    }
}

fn check_templates(
    templates: &[(Vec<PathSeg>, String)],
    scope: &ExprScope,
    used_sets: &mut BTreeSet<String>,
    used_secrets: &mut BTreeSet<String>,
    r: &mut Report,
) {
    for (p, text) in templates {
        match parse_template(text) {
            Err(e) => r.error(p, e.message),
            Ok(t) => {
                for e in t.exprs() {
                    check_expr(e, scope, p, used_sets, used_secrets, r);
                }
            }
        }
    }
}

fn check_defaults(
    suite: &Suite,
    used_sets: &mut BTreeSet<String>,
    used_secrets: &mut BTreeSet<String>,
    data: &BTreeMap<String, Option<DataInfo>>,
    r: &mut Report,
) {
    let templates: Vec<(Vec<PathSeg>, String)> = suite
        .defaults
        .http
        .headers
        .iter()
        .filter(|(_, v)| v.contains("{{"))
        .map(|(k, v)| {
            (
                vec![key("defaults"), key("http"), key("headers"), key(k)],
                v.clone(),
            )
        })
        .collect();
    let scope = ExprScope {
        suite,
        data,
        previous: None,
        all_steps: &[],
    };
    check_templates(&templates, &scope, used_sets, used_secrets, r);
}

fn jsonpath_keys(
    map: impl Iterator<Item = impl AsRef<str>>,
    p: &[PathSeg],
    what: &str,
    r: &mut Report,
) {
    for k in map {
        let k = k.as_ref();
        if !k.starts_with('$') {
            let mut path = p.to_vec();
            path.push(key(k));
            r.error(
                &path,
                format!("{what} `{k}` must be a JSONPath starting with `$`"),
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn check_scenarios(
    suite: &Suite,
    registry: &Registry,
    data: &BTreeMap<String, Option<DataInfo>>,
    env: Option<&Environment>,
    used_sets: &mut BTreeSet<String>,
    used_secrets: &mut BTreeSet<String>,
    r: &mut Report,
) {
    let mut scenario_ids = BTreeSet::new();
    let mut driven: BTreeMap<&str, &str> = BTreeMap::new();
    for (i, sc) in suite.scenarios.iter().enumerate() {
        let sp = [key("scenarios"), PathSeg::Index(i)];
        if !is_name(&sc.id) {
            r.error(
                &path(&sp, &["id"]),
                format!("scenario id `{}` must match [A-Za-z0-9_-]{{1,64}}", sc.id),
            );
        } else if !scenario_ids.insert(sc.id.as_str()) {
            r.error(
                &path(&sp, &["id"]),
                format!("scenario id `{}` is used twice", sc.id),
            );
        }
        if sc.steps.is_empty() {
            r.error(
                &path(&sp, &["steps"]),
                format!("scenario `{}` has no steps", sc.id),
            );
            continue;
        }
        let ids = sc.step_ids();
        let mut step_ids = BTreeSet::new();
        let mut previous: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for (j, step) in sc.steps.iter().enumerate() {
            let stp = path(&sp, &["steps"]);
            let stp = [stp.as_slice(), &[PathSeg::Index(j)]].concat();
            let id = &ids[j];
            if step.id.is_some() && !is_name(id) {
                r.error(
                    &path(&stp, &["id"]),
                    format!("step id `{id}` must match [A-Za-z0-9_-]{{1,64}}"),
                );
            } else if !step_ids.insert(id.clone()) {
                r.error(
                    &path(&stp, &["id"]),
                    format!("step id `{id}` is used twice in scenario `{}`", sc.id),
                );
            }
            check_step(step, &stp, registry, env, r);
            for name in step.extract.keys() {
                if !is_name(name) {
                    r.error(
                        &path(&stp, &["extract", name]),
                        format!("extract name `{name}` must match [A-Za-z0-9_-]{{1,64}}"),
                    );
                }
            }
            jsonpath_keys(
                step.extract.values(),
                &path(&stp, &["extract"]),
                "extract",
                r,
            );
            jsonpath_keys(
                step.expect.jsonpath.keys(),
                &path(&stp, &["expect", "jsonpath"]),
                "expect.jsonpath",
                r,
            );
            if let Action::KafkaConsume(c) = &step.action {
                jsonpath_keys(
                    c.match_.jsonpath.keys(),
                    &path(&stp, &["kafka_consume", "match", "jsonpath"]),
                    "match.jsonpath",
                    r,
                );
            }
            let scope = ExprScope {
                suite,
                data,
                previous: Some(&previous),
                all_steps: &ids,
            };
            check_templates(
                &step_templates(&stp, step),
                &scope,
                used_sets,
                used_secrets,
                r,
            );
            previous.insert(id.clone(), step.extract.keys().cloned().collect());
        }
        let drives = sc.drives();
        let drives_path = if sc.drives.is_some() {
            path(&sp, &["drives"])
        } else {
            sp.to_vec()
        };
        if !sc.steps.iter().any(|s| s.action.operation() == drives) {
            r.error(
                &drives_path,
                format!("drives `{drives}` is not an operation of the scenario's steps"),
            );
        }
        match driven.get(drives) {
            Some(other) => r.error(
                &drives_path,
                format!("operation `{drives}` is already driven by scenario `{other}`"),
            ),
            None => {
                driven.insert(drives, &sc.id);
            }
        }
    }
}

fn check_step(
    step: &crate::suite::schema::Step,
    stp: &[PathSeg],
    registry: &Registry,
    env: Option<&Environment>,
    r: &mut Report,
) {
    let kind = step.action.kind();
    let op_path = path(stp, &[kind.as_str(), "operation"]);
    let id = step.action.operation();
    match registry.get(id) {
        None => r.error(&op_path, format!("unknown operation `{id}`")),
        Some(op) => {
            if op.draft.kind != kind {
                r.error(
                    &op_path,
                    format!(
                        "operation `{id}` is `{}`, but the step is `{}`",
                        op.draft.kind.as_str(),
                        kind.as_str()
                    ),
                );
            } else {
                if op.status == Status::Removed {
                    r.warning(
                        &op_path,
                        format!("operation `{id}` is removed from the registry"),
                    );
                }
                if step.expect.schema.is_some() && op.draft.response.is_none() {
                    r.warning(&path(stp, &["expect", "schema"]), format!("operation `{id}` has no response schema in the registry; `schema: response` cannot be checked"));
                }
            }
        }
    }
    if let Some(StatusExpect::Class(c)) = &step.expect.status
        && !StatusExpect::is_valid_class(c)
    {
        r.error(
            &path(stp, &["expect", "status"]),
            format!("status class `{c}` must be 1xx..5xx"),
        );
    }
    match &step.action {
        Action::Http(h) => {
            if h.body.is_some() && h.body_text.is_some() {
                r.error(
                    &path(stp, &["http", "body_text"]),
                    "use either `body` or `body_text`, not both".into(),
                );
            }
            if h.timeout_ms == Some(0) {
                r.error(
                    &path(stp, &["http", "timeout_ms"]),
                    "timeout_ms must be at least 1".into(),
                );
            }
        }
        Action::KafkaProduce(k) => {
            if k.value.is_some() && k.value_text.is_some() {
                r.error(
                    &path(stp, &["kafka_produce", "value_text"]),
                    "use either `value` or `value_text`, not both".into(),
                );
            }
            if k.encoding == Encoding::Avro {
                let p = path(stp, &["kafka_produce", "encoding"]);
                match env {
                    None => r.warning(&p, "`encoding: avro` needs `schema_registry` in the environment; no environment was given".into()),
                    Some(e) if e.schema_registry.is_none() => r.error(&p, format!("environment `{}` has no `schema_registry`, required by `encoding: avro`", e.id)),
                    Some(_) => {}
                }
            }
        }
        Action::KafkaConsume(c) => {
            if c.timeout_ms == 0 {
                r.error(
                    &path(stp, &["kafka_consume", "timeout_ms"]),
                    "timeout_ms must be at least 1".into(),
                );
            }
            if step.expect.status.is_some() {
                r.error(
                    &path(stp, &["expect", "status"]),
                    "`expect.status` does not apply to kafka_consume".into(),
                );
            }
        }
    }
    if kind == OpKind::KafkaProduce && (!step.expect.is_empty() || !step.extract.is_empty()) {
        r.error(
            &path(stp, &["expect"]),
            "kafka_produce has no response: `expect` and `extract` do not apply".into(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::{EnvKind, Manifest};
    use crate::suite::coverage::tests::fixture;

    fn registry() -> Registry {
        crate::from_yaml(&fixture("project/registry/operations.yaml")).unwrap()
    }

    fn data() -> BTreeMap<String, Option<DataInfo>> {
        BTreeMap::from([
            (
                "users".to_string(),
                Some(DataInfo {
                    columns: vec!["id".into(), "email".into(), "tier".into()],
                    rows: 5,
                }),
            ),
            (
                "skus".to_string(),
                Some(DataInfo {
                    columns: vec!["sku".into(), "price".into()],
                    rows: 4,
                }),
            ),
        ])
    }

    fn env() -> Environment {
        let m: Manifest = crate::from_yaml(&fixture("project/glafira-load.yaml")).unwrap();
        m.environments.into_iter().find(|e| e.id == "test").unwrap()
    }

    fn run(text: &str) -> (Option<Suite>, Vec<Finding>) {
        check(text, "suite.yaml", &registry(), &data(), Some(&env()))
    }

    fn errors(findings: &[Finding]) -> Vec<(Option<u32>, &str)> {
        findings
            .iter()
            .filter(|f| f.level == Level::Error)
            .map(|f| (f.line, f.message.as_str()))
            .collect()
    }

    fn warnings(findings: &[Finding]) -> Vec<(Option<u32>, &str)> {
        findings
            .iter()
            .filter(|f| f.level == Level::Warning)
            .map(|f| (f.line, f.message.as_str()))
            .collect()
    }

    #[test]
    fn sample_suite_is_clean() {
        let (suite, findings) = run(&fixture("orders/suite.yaml"));
        assert!(suite.is_some());
        assert_eq!(findings, vec![], "{findings:?}");
        let (suite, findings) = check(
            &fixture("orders/suite.yaml"),
            "suite.yaml",
            &registry(),
            &BTreeMap::new(),
            None,
        );
        assert!(suite.is_some());
        assert_eq!(
            findings,
            vec![],
            "без сведений о файлах колонки не проверяются"
        );
    }

    #[test]
    fn unknown_field_fixture() {
        let (suite, f) = run(&fixture("invalid/unknown-field.yaml"));
        assert!(suite.is_none());
        assert_eq!(f.len(), 1);
        assert_eq!(
            (f[0].level, f[0].line, f[0].file.as_str()),
            (Level::Error, Some(6), "suite.yaml")
        );
        assert!(
            f[0].message.contains("unknown field `expct`"),
            "{}",
            f[0].message
        );
    }

    #[test]
    fn missing_operation_fixture() {
        let (suite, f) = run(&fixture("invalid/missing-operation.yaml"));
        assert!(suite.is_some());
        assert_eq!(
            errors(&f),
            vec![(Some(7), "unknown operation `http:orders:POST /v1/refunds`")]
        );
        assert!(warnings(&f).is_empty());
    }

    #[test]
    fn bad_expression_fixture() {
        let (_, f) = run(&fixture("invalid/bad-expression.yaml"));
        assert_eq!(
            errors(&f),
            vec![(Some(8), "unknown expression `random 1 5`")]
        );
    }

    #[test]
    fn missing_column_fixture() {
        let (_, f) = run(&fixture("invalid/missing-column.yaml"));
        assert_eq!(
            errors(&f),
            vec![(Some(10), "data set `users` has no column `phone`")]
        );
        let mut no_file = data();
        no_file.insert("users".into(), None);
        let (_, f) = check(
            &fixture("invalid/missing-column.yaml"),
            "suite.yaml",
            &registry(),
            &no_file,
            None,
        );
        assert_eq!(
            errors(&f),
            vec![(Some(4), "data set `users`: file `data/users.csv` not found")]
        );
    }

    #[test]
    fn future_step_fixture() {
        let (_, f) = run(&fixture("invalid/future-step.yaml"));
        assert_eq!(
            errors(&f),
            vec![(
                Some(7),
                "step `create` runs later: only previous steps can be referenced"
            )]
        );
    }

    #[test]
    fn undeclared_secret_fixture() {
        let (_, f) = run(&fixture("invalid/undeclared-secret.yaml"));
        assert_eq!(
            errors(&f),
            vec![(Some(9), "secret `ADMIN_TOKEN` is not declared in `secrets`")]
        );
        assert_eq!(
            warnings(&f),
            vec![(Some(3), "secret `API_TOKEN` is declared but not used")]
        );
    }

    #[test]
    fn duplicate_drives_fixture() {
        let (_, f) = run(&fixture("invalid/duplicate-drives.yaml"));
        assert_eq!(
            errors(&f),
            vec![(
                Some(8),
                "operation `http:orders:POST /v1/orders` is already driven by scenario `create`"
            )]
        );
    }

    const BASE: &str = "format: 1\nname: orders\nscenarios:\n  - id: a\n    steps:\n      - http: { operation: \"http:orders:POST /v1/orders\" }\n";

    #[test]
    fn header_and_structure_errors() {
        let (_, f) = run(
            "format: 2\nname: Bad Name\nscenarios: []\ndefaults: { max_in_flight: 0, http: { timeout_ms: 0 } }\n",
        );
        assert_eq!(
            errors(&f),
            vec![
                (Some(1), "unsupported suite format 2, expected 1"),
                (Some(2), "suite name `Bad Name` must match [a-z0-9-]{1,64}"),
                (Some(3), "suite has no scenarios"),
                (Some(4), "max_in_flight must be at least 1"),
                (Some(4), "timeout_ms must be at least 1"),
            ]
        );
        let (_, f) = run(
            "format: 1\nname: x\nscenarios:\n  - id: a\n    steps: []\n  - id: a\n    steps:\n      - id: s\n        http: { operation: \"http:orders:POST /v1/orders\" }\n      - id: s\n        http: { operation: \"http:orders:GET /v1/orders\" }\n      - http: { operation: \"http:orders:GET /v1/health\" }\n      - id: step3\n        http: { operation: \"http:orders:GET /v1/health\" }\n",
        );
        assert_eq!(
            errors(&f),
            vec![
                (Some(5), "scenario `a` has no steps"),
                (Some(6), "scenario id `a` is used twice"),
                (Some(10), "step id `s` is used twice in scenario `a`"),
                (Some(13), "step id `step3` is used twice in scenario `a`"),
            ]
        );
    }

    #[test]
    fn operation_kind_and_removed_status() {
        let text = "format: 1\nname: x\nscenarios:\n  - id: a\n    steps:\n      - http: { operation: \"kafka_consume:orders.created\" }\n      - kafka_produce: { operation: \"http:orders:POST /v1/orders\" }\n      - http: { operation: \"http:orders:DELETE /v1/orders/{id}\" }\n      - http: { operation: \"http:orders:GET /v1/health\" }\n        expect: { schema: response }\n";
        let (_, f) = run(text);
        assert_eq!(
            errors(&f),
            vec![
                (
                    Some(6),
                    "operation `kafka_consume:orders.created` is `kafka_consume`, but the step is `http`"
                ),
                (
                    Some(7),
                    "operation `http:orders:POST /v1/orders` is `http`, but the step is `kafka_produce`"
                ),
            ]
        );
        assert_eq!(
            warnings(&f),
            vec![
                (
                    Some(8),
                    "operation `http:orders:DELETE /v1/orders/{id}` is removed from the registry"
                ),
                (
                    Some(10),
                    "operation `http:orders:GET /v1/health` has no response schema in the registry; `schema: response` cannot be checked"
                ),
            ]
        );
    }

    #[test]
    fn drives_must_be_a_step_operation() {
        let (_, f) = run(
            "format: 1\nname: x\nscenarios:\n  - id: a\n    drives: \"http:orders:GET /v1/orders\"\n    steps:\n      - http: { operation: \"http:orders:POST /v1/orders\" }\n",
        );
        assert_eq!(
            errors(&f),
            vec![(
                Some(5),
                "drives `http:orders:GET /v1/orders` is not an operation of the scenario's steps"
            )]
        );
    }

    #[test]
    fn data_sets_reserved_names_paths_and_unused() {
        let (_, f) = run(
            "format: 1\nname: x\ndata:\n  env: data/a.csv\n  'bad name': data/b.csv\n  up: ../x.csv\n  abs: /tmp/x.csv\n  win: 'C:\\x.csv'\nscenarios:\n  - id: a\n    steps:\n      - http: { operation: \"http:orders:POST /v1/orders\", body: { a: \"{{nope.id}}\", b: \"{{pick up.id}}\" } }\n",
        );
        assert_eq!(
            errors(&f),
            vec![
                (
                    Some(7),
                    "data set `abs`: path `/tmp/x.csv` must be relative and inside the project"
                ),
                (
                    Some(5),
                    "data set name `bad name` must match [A-Za-z0-9_-]{1,64}"
                ),
                (Some(4), "data set name `env` is reserved"),
                (
                    Some(6),
                    "data set `up`: path `../x.csv` must be relative and inside the project"
                ),
                (
                    Some(8),
                    "data set `win`: path `C:\\x.csv` must be relative and inside the project"
                ),
                (Some(12), "unknown data set `nope`"),
            ]
        );
        let unused: Vec<&str> = warnings(&f).iter().map(|w| w.1).collect();
        assert_eq!(
            unused,
            vec![
                "data set `abs` is not used",
                "data set `bad name` is not used",
                "data set `env` is not used",
                "data set `win` is not used"
            ]
        );
    }

    #[test]
    fn step_references_need_the_extract_name_and_defaults_cannot_use_steps() {
        let (_, f) = run(
            "format: 1\nname: x\ndefaults:\n  http:\n    headers: { X-Order: \"{{steps.create.order_id}}\", X-Env: \"{{env.http}}\" }\nscenarios:\n  - id: a\n    steps:\n      - id: create\n        http: { operation: \"http:orders:POST /v1/orders\" }\n        extract: { order_id: \"$.id\" }\n      - http:\n          operation: \"http:orders:GET /v1/orders/{id}\"\n          path: { id: \"{{steps.create.total}}\", other: \"{{steps.nope.x}}\" }\n",
        );
        assert_eq!(
            errors(&f),
            vec![
                (Some(5), "`steps.*` is not available in defaults"),
                (Some(14), "step `create` has no extract `total`"),
                (Some(14), "unknown step `nope`"),
            ]
        );
    }

    #[test]
    fn secrets_duplicates_and_names() {
        let (_, f) = run(
            "format: 1\nname: x\nsecrets: [A, A, 'b c']\nscenarios:\n  - id: a\n    steps:\n      - http: { operation: \"http:orders:POST /v1/orders\", headers: { Authorization: \"{{secret A}}\" } }\n",
        );
        assert_eq!(
            errors(&f),
            vec![
                (Some(3), "secret `A` is declared twice"),
                (Some(3), "secret name `b c` must match [A-Za-z0-9_-]{1,64}")
            ]
        );
    }

    #[test]
    fn avro_needs_a_schema_registry() {
        let text = "format: 1\nname: x\nscenarios:\n  - id: a\n    steps:\n      - kafka_produce:\n          operation: \"kafka_produce:orders.paid\"\n          value: { orderId: \"1\" }\n          encoding: avro\n";
        let (_, f) = run(text);
        assert!(f.is_empty(), "{f:?}");
        let (_, f) = check(text, "suite.yaml", &registry(), &data(), None);
        assert_eq!(
            warnings(&f),
            vec![(
                Some(9),
                "`encoding: avro` needs `schema_registry` in the environment; no environment was given"
            )]
        );
        let mut e = env();
        e.schema_registry = None;
        e.kind = EnvKind::Dev;
        let (_, f) = check(text, "suite.yaml", &registry(), &data(), Some(&e));
        assert_eq!(
            errors(&f),
            vec![(
                Some(9),
                "environment `test` has no `schema_registry`, required by `encoding: avro`"
            )]
        );
    }

    #[test]
    fn step_shape_errors() {
        let text = "format: 1\nname: x\nscenarios:\n  - id: a\n    steps:\n      - http: { operation: \"http:orders:POST /v1/orders\", body: { a: 1 }, body_text: \"x\", timeout_ms: 0 }\n        expect: { status: 6xx, jsonpath: { id: exists } }\n        extract: { \"bad.name\": \"$.id\", ok: \"id\" }\n      - kafka_produce: { operation: \"kafka_produce:orders.paid\", value: { a: 1 }, value_text: \"x\" }\n        expect: { max_ms: 5 }\n      - kafka_consume: { operation: \"kafka_consume:orders.created\", timeout_ms: 0, match: { jsonpath: { orderId: \"1\" } } }\n        expect: { status: 200 }\n";
        let (_, f) = run(text);
        let msgs: Vec<&str> = errors(&f).iter().map(|e| e.1).collect();
        assert_eq!(
            msgs,
            vec![
                "status class `6xx` must be 1xx..5xx",
                "use either `body` or `body_text`, not both",
                "timeout_ms must be at least 1",
                "extract name `bad.name` must match [A-Za-z0-9_-]{1,64}",
                "extract `id` must be a JSONPath starting with `$`",
                "expect.jsonpath `id` must be a JSONPath starting with `$`",
                "use either `value` or `value_text`, not both",
                "kafka_produce has no response: `expect` and `extract` do not apply",
                "timeout_ms must be at least 1",
                "`expect.status` does not apply to kafka_consume",
                "match.jsonpath `orderId` must be a JSONPath starting with `$`",
            ]
        );
        assert!(
            errors(&f).iter().all(|e| e.0.is_some()),
            "у каждой находки есть строка: {f:?}"
        );
    }

    #[test]
    fn base_suite_is_clean_and_findings_serialize_snake_case() {
        let (_, f) = run(BASE);
        assert!(f.is_empty(), "{f:?}");
        let text = serde_json::to_string(&Finding {
            level: Level::Warning,
            file: "s".into(),
            line: Some(3),
            message: "m".into(),
        })
        .unwrap();
        assert_eq!(
            text,
            r#"{"level":"warning","file":"s","line":3,"message":"m"}"#
        );
    }
}
