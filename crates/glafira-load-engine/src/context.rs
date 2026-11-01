//! Контекст прогона и итерации: наборы данных, секреты, адреса стенда, значения `extract`
//! (спека L2 §3.1, §3.3).

use std::collections::BTreeMap;
use std::path::{Component, Path};
use std::time::{SystemTime, UNIX_EPOCH};

use glafira_load_model::suite::Suite;
use glafira_load_model::suite::expr::Context;
use rand::Rng;
use serde_json::Value;

use crate::agent_api::EnvAddresses;

/// Таблица набора данных: имена колонок и строки в их порядке.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DataTable {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
}

impl DataTable {
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn column(&self, name: &str) -> Option<usize> {
        self.columns.iter().position(|c| c == name)
    }

    /// Значение ячейки; `None` — нет колонки или строки.
    pub fn value(&self, row: usize, col: &str) -> Option<&Value> {
        let idx = self.column(col)?;
        self.rows.get(row)?.get(idx)
    }
}

/// Формат файла данных по расширению.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataFormat {
    Csv,
    JsonLines,
}

impl DataFormat {
    pub fn from_path(path: &str) -> Result<Self, String> {
        let ext = Path::new(path)
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .unwrap_or_default();
        match ext.as_str() {
            "csv" => Ok(DataFormat::Csv),
            "jsonl" | "ndjson" => Ok(DataFormat::JsonLines),
            _ => Err(format!(
                "unsupported data file `{path}`: expected .csv or .jsonl"
            )),
        }
    }
}

/// Разбор текста набора: CSV с заголовком или JSON Lines (объект на строку; колонки —
/// объединение ключей в порядке первого появления, отсутствующий ключ — `null`).
pub fn parse_table(format: DataFormat, text: &str) -> Result<DataTable, String> {
    match format {
        DataFormat::Csv => {
            let mut reader = csv::ReaderBuilder::new()
                .has_headers(true)
                .from_reader(text.as_bytes());
            let columns: Vec<String> = reader
                .headers()
                .map_err(|e| format!("bad csv header: {e}"))?
                .iter()
                .map(|h| h.trim().to_string())
                .collect();
            let mut rows = Vec::new();
            for (i, rec) in reader.records().enumerate() {
                let rec = rec.map_err(|e| format!("csv line {}: {e}", i + 2))?;
                rows.push(
                    rec.iter()
                        .map(|cell| Value::String(cell.to_string()))
                        .collect(),
                );
            }
            Ok(DataTable { columns, rows })
        }
        DataFormat::JsonLines => {
            let mut columns: Vec<String> = Vec::new();
            let mut objects: Vec<serde_json::Map<String, Value>> = Vec::new();
            for (i, line) in text.lines().enumerate() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                let value: Value =
                    serde_json::from_str(line).map_err(|e| format!("jsonl line {}: {e}", i + 1))?;
                let Value::Object(map) = value else {
                    return Err(format!("jsonl line {}: expected a JSON object", i + 1));
                };
                for key in map.keys() {
                    if !columns.iter().any(|c| c == key) {
                        columns.push(key.clone());
                    }
                }
                objects.push(map);
            }
            let rows = objects
                .into_iter()
                .map(|mut map| {
                    columns
                        .iter()
                        .map(|c| map.remove(c).unwrap_or(Value::Null))
                        .collect()
                })
                .collect();
            Ok(DataTable { columns, rows })
        }
    }
}

/// Разбор набора данных из памяти (бандл агента): формат — по расширению `file_name`
/// (путь из `suite.data[*].path`), содержимое — UTF-8.
pub fn parse_data(file_name: &str, bytes: &[u8]) -> Result<DataTable, String> {
    let format = DataFormat::from_path(file_name)?;
    let text =
        std::str::from_utf8(bytes).map_err(|e| format!("`{file_name}` is not UTF-8: {e}"))?;
    parse_table(format, text)
}

/// Колонки и число строк файла данных для `suite::check`: разбор тот же, что у прогона
/// (`parse_data`), колонки JSON Lines — по алфавиту. `path` — относительно `root`, как в
/// `suite.data[*].path`.
///
/// `Some` — только если файл внутри проекта и разобран целиком. `None` — путь вне проекта
/// (об этом сообщает `check` кодом `PATH_OUTSIDE_PROJECT`), файла нет, или он испорчен
/// (не UTF-8, битая строка CSV или JSON Lines, неизвестное расширение): испорченный файл
/// пишется в журнал, а `check` сообщает о нём как об отсутствующем наборе.
pub fn data_info(root: &Path, path: &str) -> Option<glafira_load_model::suite::check::DataInfo> {
    if !is_inside_project(path) {
        return None;
    }
    let bytes = std::fs::read(root.join(path)).ok()?;
    let table = match parse_data(path, &bytes) {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!("data file `{path}` is not readable: {e}");
            return None;
        }
    };
    let mut columns = table.columns;
    if DataFormat::from_path(path) == Ok(DataFormat::JsonLines) {
        columns.sort();
    }
    Some(glafira_load_model::suite::check::DataInfo {
        columns,
        rows: table.rows.len(),
    })
}

/// Путь внутри проекта: относительный, без `..`.
pub fn is_inside_project(path: &str) -> bool {
    let p = Path::new(path);
    !path.is_empty()
        && !p.is_absolute()
        && p.components()
            .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
}

/// Читает все наборы сьюта из папки проекта.
pub fn load_data(root: &Path, suite: &Suite) -> Result<BTreeMap<String, DataTable>, String> {
    let mut out = BTreeMap::new();
    for (name, set) in &suite.data {
        if !is_inside_project(&set.path) {
            return Err(format!(
                "data `{name}`: path `{}` is outside the project",
                set.path
            ));
        }
        let format = DataFormat::from_path(&set.path).map_err(|e| format!("data `{name}`: {e}"))?;
        let text = std::fs::read_to_string(root.join(&set.path))
            .map_err(|e| format!("data `{name}`: cannot read `{}`: {e}", set.path))?;
        let table = parse_table(format, &text).map_err(|e| format!("data `{name}`: {e}"))?;
        out.insert(name.clone(), table);
    }
    Ok(out)
}

/// Неизменяемая часть контекста на весь прогон агента.
#[derive(Debug, Clone, Default)]
pub struct RunContext {
    pub run_id: String,
    pub agent_id: String,
    pub env: EnvAddresses,
    pub secrets: BTreeMap<String, String>,
    pub data: BTreeMap<String, DataTable>,
}

impl RunContext {
    /// Значения секретов для маскирования примеров.
    pub fn secret_values(&self) -> Vec<String> {
        self.secrets.values().cloned().collect()
    }
}

/// Контекст одной итерации: текущие строки наборов (по кругу от номера итерации),
/// значения `extract` по шагам и шаг, который исполняется сейчас.
#[derive(Debug)]
pub struct IterationCtx<'a> {
    run: &'a RunContext,
    rows: BTreeMap<String, usize>,
    steps: BTreeMap<String, BTreeMap<String, Value>>,
    current_step: String,
}

impl<'a> IterationCtx<'a> {
    pub fn new(run: &'a RunContext, iteration: u64) -> Self {
        let rows = run
            .data
            .iter()
            .filter(|(_, t)| !t.is_empty())
            .map(|(name, t)| (name.clone(), (iteration % t.len() as u64) as usize))
            .collect();
        Self {
            run,
            rows,
            steps: BTreeMap::new(),
            current_step: String::new(),
        }
    }

    pub fn run(&self) -> &'a RunContext {
        self.run
    }

    /// Планировщик зовёт перед каждым шагом: под этим id исполнитель сохраняет `extract`.
    pub fn begin_step(&mut self, id: &str) {
        self.current_step = id.to_string();
        self.steps.entry(self.current_step.clone()).or_default();
    }

    pub fn current_step(&self) -> &str {
        &self.current_step
    }

    /// Сохраняет значение `extract` текущего шага.
    pub fn store(&mut self, name: &str, value: Value) {
        self.steps
            .entry(self.current_step.clone())
            .or_default()
            .insert(name.to_string(), value);
    }

    pub fn steps(&self) -> &BTreeMap<String, BTreeMap<String, Value>> {
        &self.steps
    }
}

impl Context for IterationCtx<'_> {
    fn data(&self, set: &str, col: &str) -> Option<Value> {
        let row = *self.rows.get(set)?;
        self.run.data.get(set)?.value(row, col).cloned()
    }

    fn pick(&self, set: &str, col: &str) -> Option<Value> {
        let table = self.run.data.get(set)?;
        if table.is_empty() {
            return None;
        }
        let row = rand::rng().random_range(0..table.len());
        table.value(row, col).cloned()
    }

    fn step(&self, step: &str, name: &str) -> Option<Value> {
        self.steps.get(step)?.get(name).cloned()
    }

    fn secret(&self, name: &str) -> Option<String> {
        self.run.secrets.get(name).cloned()
    }

    fn env(&self, key: &str) -> Option<String> {
        match key {
            "http" => self.run.env.http.clone(),
            "kafka" => self.run.env.kafka.clone(),
            "schema_registry" => self.run.env.schema_registry.clone(),
            _ => None,
        }
    }

    fn run_id(&self) -> &str {
        &self.run.run_id
    }

    fn agent_id(&self) -> &str {
        &self.run.agent_id
    }

    fn random_u64(&self) -> u64 {
        rand::rng().random::<u64>()
    }

    fn now_unix_ms(&self) -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glafira_load_model::suite::expr::{parse_template, render, render_string};
    use glafira_load_model::suite::{DataMode, DataSet, Defaults};

    fn suite_with(data: &[(&str, &str)]) -> Suite {
        Suite {
            format: 1,
            name: "t".into(),
            secrets: vec![],
            data: data
                .iter()
                .map(|(n, p)| {
                    (
                        n.to_string(),
                        DataSet {
                            path: p.to_string(),
                            mode: DataMode::Shared,
                        },
                    )
                })
                .collect(),
            defaults: Defaults::default(),
            scenarios: vec![],
        }
    }

    #[test]
    fn csv_and_jsonl_are_parsed_into_tables() {
        let csv = parse_table(DataFormat::Csv, "id, name\n1,Ann\n2,Bob\n").unwrap();
        assert_eq!(csv.columns, ["id", "name"]);
        assert_eq!(csv.rows.len(), 2);
        assert_eq!(csv.value(1, "name"), Some(&Value::String("Bob".into())));
        assert_eq!(csv.value(1, "nope"), None);
        assert_eq!(csv.value(5, "id"), None);
        let jsonl = parse_table(
            DataFormat::JsonLines,
            "{\"sku\":\"A\",\"qty\":1}\n\n{\"sku\":\"B\",\"price\":2.5}\n",
        )
        .unwrap();
        let mut cols = jsonl.columns.clone();
        cols.sort();
        assert_eq!(cols, ["price", "qty", "sku"]);
        assert_eq!(jsonl.value(0, "qty"), Some(&Value::from(1)));
        assert_eq!(jsonl.value(0, "price"), Some(&Value::Null));
        assert_eq!(jsonl.value(1, "price"), Some(&Value::from(2.5)));
        assert!(
            parse_table(DataFormat::JsonLines, "[1,2]\n")
                .unwrap_err()
                .contains("line 1")
        );
        assert!(parse_table(DataFormat::Csv, "a,b\n1\n").is_err());
        assert_eq!(
            DataFormat::from_path("data/x.CSV").unwrap(),
            DataFormat::Csv
        );
        assert_eq!(
            DataFormat::from_path("data/x.ndjson").unwrap(),
            DataFormat::JsonLines
        );
        assert!(DataFormat::from_path("data/x.txt").is_err());
    }

    #[test]
    fn load_data_reads_files_inside_the_project_only() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("data")).unwrap();
        std::fs::write(dir.path().join("data/users.csv"), "id\n1\n2\n3\n").unwrap();
        std::fs::write(dir.path().join("data/skus.jsonl"), "{\"sku\":\"A\"}\n").unwrap();
        let data = load_data(
            dir.path(),
            &suite_with(&[("users", "data/users.csv"), ("skus", "data/skus.jsonl")]),
        )
        .unwrap();
        assert_eq!(data["users"].len(), 3);
        assert_eq!(data["skus"].len(), 1);
        let err = load_data(dir.path(), &suite_with(&[("u", "../users.csv")])).unwrap_err();
        assert!(err.contains("outside the project"), "{err}");
        let err = load_data(dir.path(), &suite_with(&[("u", "data/none.csv")])).unwrap_err();
        assert!(err.contains("cannot read"), "{err}");
        assert!(!is_inside_project("/etc/passwd"));
        assert!(is_inside_project("./data/a.csv"));
    }

    fn run_ctx() -> RunContext {
        RunContext {
            run_id: "run-1".into(),
            agent_id: "gen-1".into(),
            env: EnvAddresses {
                http: Some("http://orders.test".into()),
                kafka: Some("kafka:9092".into()),
                schema_registry: None,
            },
            secrets: BTreeMap::from([("API_TOKEN".to_string(), "s3cr3t".to_string())]),
            data: BTreeMap::from([(
                "users".to_string(),
                parse_table(DataFormat::Csv, "id,name\n1,Ann\n2,Bob\n3,Cid\n").unwrap(),
            )]),
        }
    }

    #[test]
    fn iteration_context_walks_rows_in_a_circle_and_serves_expressions() {
        let run = run_ctx();
        let ids: Vec<String> = (0..5)
            .map(|i| {
                let ctx = IterationCtx::new(&run, i);
                render_string(&parse_template("{{users.id}}").unwrap(), &ctx).unwrap()
            })
            .collect();
        assert_eq!(ids, ["1", "2", "3", "1", "2"]);
        let mut ctx = IterationCtx::new(&run, 1);
        ctx.begin_step("create");
        ctx.store("order_id", Value::from(42));
        assert_eq!(ctx.current_step(), "create");
        assert_eq!(ctx.step("create", "order_id"), Some(Value::from(42)));
        assert_eq!(ctx.step("create", "none"), None);
        assert_eq!(ctx.step("other", "order_id"), None);
        assert_eq!(
            render(&parse_template("{{steps.create.order_id}}").unwrap(), &ctx).unwrap(),
            Value::from(42),
            "одно выражение сохраняет тип"
        );
        assert_eq!(ctx.secret("API_TOKEN").as_deref(), Some("s3cr3t"));
        assert_eq!(ctx.secret("NOPE"), None);
        assert_eq!(ctx.env("http").as_deref(), Some("http://orders.test"));
        assert_eq!(ctx.env("kafka").as_deref(), Some("kafka:9092"));
        assert_eq!(ctx.env("schema_registry"), None);
        assert_eq!(ctx.env("grpc"), None);
        assert_eq!((ctx.run_id(), ctx.agent_id()), ("run-1", "gen-1"));
        assert!(ctx.now_unix_ms() > 1_700_000_000_000);
        let picked = ctx.pick("users", "name").unwrap();
        assert!(["Ann", "Bob", "Cid"].contains(&picked.as_str().unwrap()));
        assert_eq!(ctx.pick("none", "name"), None);
        assert_eq!(ctx.data("users", "nope"), None);
        let a = ctx.random_u64();
        let b = ctx.random_u64();
        assert!(a != b || ctx.random_u64() != a);
        assert_eq!(run.secret_values(), ["s3cr3t"]);
    }

    #[test]
    fn empty_table_gives_no_row() {
        let mut run = run_ctx();
        run.data.insert("empty".into(), DataTable::default());
        let ctx = IterationCtx::new(&run, 0);
        assert_eq!(ctx.data("empty", "id"), None);
        assert_eq!(ctx.pick("empty", "id"), None);
        assert_eq!(ctx.data("users", "id"), Some(Value::String("1".into())));
    }

    #[test]
    fn parse_data_picks_format_by_extension() {
        let csv = parse_data("data/users.csv", b"id,name\n1,Ann\n").unwrap();
        assert_eq!(
            (csv.columns.as_slice(), csv.rows.len()),
            (["id".to_string(), "name".to_string()].as_slice(), 1)
        );
        let jsonl = parse_data("data/skus.jsonl", b"{\"sku\": \"A\", \"price\": 10}\n").unwrap();
        // Порядок колонок JSON Lines зависит от `preserve_order` у serde_json (объединение
        // features в workspace), поэтому значения сверяются по именам колонок.
        assert_eq!(
            (jsonl.value(0, "sku"), jsonl.value(0, "price")),
            (Some(&serde_json::json!("A")), Some(&serde_json::json!(10)))
        );
        assert!(
            parse_data("data/x.txt", b"")
                .unwrap_err()
                .contains("unsupported data file")
        );
        assert!(
            parse_data("data/x.csv", &[0xff, 0xfe])
                .unwrap_err()
                .contains("not UTF-8")
        );
    }

    #[test]
    fn data_info_reads_csv_and_jsonl_columns_and_rows() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("data")).unwrap();
        std::fs::write(
            dir.path().join("data/users.csv"),
            "id,name\n1,\"a, b\"\n2,c\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("data/skus.jsonl"),
            "{\"sku\":\"x\",\"qty\":1}\n\n{\"sku\":\"y\"}\n",
        )
        .unwrap();
        let users = data_info(dir.path(), "data/users.csv").unwrap();
        assert_eq!(
            (users.columns, users.rows),
            (vec!["id".to_string(), "name".to_string()], 2)
        );
        let skus = data_info(dir.path(), "data/skus.jsonl").unwrap();
        assert_eq!(
            (skus.columns, skus.rows),
            (vec!["qty".to_string(), "sku".to_string()], 2)
        );
        assert!(data_info(dir.path(), "data/none.csv").is_none());
        // Испорченный файл целиком не разобран: `None`, `check` покажет его как отсутствующий.
        std::fs::write(dir.path().join("data/bad.csv"), "a,b\n1\n").unwrap();
        assert!(data_info(dir.path(), "data/bad.csv").is_none());
        std::fs::write(dir.path().join("data/bad.jsonl"), "{\"a\":1}\nnot json\n").unwrap();
        assert!(data_info(dir.path(), "data/bad.jsonl").is_none());
        // Путь вне проекта не читается, даже если файл существует.
        std::fs::write(dir.path().join("outside.csv"), "id\n1\n").unwrap();
        let nested = dir.path().join("data");
        assert!(data_info(&nested, "../outside.csv").is_none());
        assert!(
            data_info(
                &nested,
                &dir.path().join("outside.csv").display().to_string()
            )
            .is_none()
        );
    }
}
