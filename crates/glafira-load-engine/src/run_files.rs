//! Файлы прогона `runs/<дата>/` (спека L2 §6): `run.yaml`, `metrics.jsonl`, `phases.yaml`,
//! `errors.jsonl`, `summary.yaml`, `smoke.yaml`; файлы агента `hist.bin` (interval log).

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use base64::Engine as _;
use hdrhistogram::Histogram;
use hdrhistogram::serialization::interval_log::{
    IntervalLogIterator, IntervalLogWriterBuilder, LogEntry, Tag,
};
use hdrhistogram::serialization::{Deserializer, V2Serializer};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::agent_api::{AgentSummary, ErrorSample, Mode, PhaseEvent, RunStatus, Tick};
use crate::errors::SampleCap;
use crate::metrics::{OpTotals, ScTotals, Totals, new_histogram};
use crate::summary::{SmokeReport, sorted_summary};

pub const RUNS_DIR: &str = "runs";
pub const RUN_FILE: &str = "run.yaml";
pub const METRICS_FILE: &str = "metrics.jsonl";
pub const PHASES_FILE: &str = "phases.yaml";
pub const ERRORS_FILE: &str = "errors.jsonl";
pub const SUMMARY_FILE: &str = "summary.yaml";
pub const SMOKE_FILE: &str = "smoke.yaml";
pub const HIST_FILE: &str = "hist.bin";
pub const AGENTS_DIR: &str = "agents";
/// Префикс тега сценария в `hist.bin`; id операций начинаются с `http:`, `kafka_produce:`, `kafka_consume:`
/// и с ним не пересекаются.
pub const SCENARIO_TAG: &str = "scenario:";

fn one() -> u32 {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RunAgent {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default = "one")]
    pub weight: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

/// `run.yaml`: пишется при создании папки и дописывается `ended_at`/`status` в `finish`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RunMeta {
    pub run_id: String,
    pub suite: String,
    pub profile: String,
    pub env: String,
    pub mode: Mode,
    pub agents: Vec<RunAgent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_commit: Option<String>,
    /// Сервис → версия из реестра (`last_seen.version`).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub services: BTreeMap<String, String>,
    /// RFC 3339, UTC.
    pub started_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<RunStatus>,
}

/// Строка `metrics.jsonl`: операция (`op`) или сценарий (`scenario`) за секунду `t`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MetricLine {
    pub t: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub op: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scenario: Option<String>,
    pub n: u64,
    #[serde(default)]
    pub err: BTreeMap<String, u64>,
    pub dropped: u64,
    pub p50: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub p90: Option<f64>,
    pub p95: f64,
    pub p99: f64,
    pub max: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes_in: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes_out: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lag: Option<i64>,
}

/// Строки `metrics.jsonl` одного тика: сначала операции, потом сценарии.
pub fn tick_lines(t: &Tick) -> Vec<MetricLine> {
    let mut out = Vec::with_capacity(t.ops.len() + t.scenarios.len());
    for o in &t.ops {
        out.push(MetricLine {
            t: t.t,
            op: Some(o.id.clone()),
            scenario: None,
            n: o.n,
            err: o.err.clone(),
            dropped: o.dropped,
            p50: o.p50,
            p90: Some(o.p90),
            p95: o.p95,
            p99: o.p99,
            max: o.max,
            bytes_in: Some(o.bytes_in),
            bytes_out: Some(o.bytes_out),
            lag: o.lag,
        });
    }
    for s in &t.scenarios {
        out.push(MetricLine {
            t: t.t,
            op: None,
            scenario: Some(s.id.clone()),
            n: s.n,
            err: s.err.clone(),
            dropped: s.dropped,
            p50: s.p50,
            p90: None,
            p95: s.p95,
            p99: s.p99,
            max: s.max,
            bytes_in: None,
            bytes_out: None,
            lag: None,
        });
    }
    out
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct PhasesFile {
    pub phases: Vec<PhaseEvent>,
}

/// Имя папки прогона: старт в UTC, `2026-11-02T14-30-00` или `smoke-…`.
pub fn run_dir_name(started_at: OffsetDateTime, smoke: bool) -> String {
    let t = started_at.to_offset(time::UtcOffset::UTC);
    let stamp = format!(
        "{:04}-{:02}-{:02}T{:02}-{:02}-{:02}",
        t.year(),
        u8::from(t.month()),
        t.day(),
        t.hour(),
        t.minute(),
        t.second()
    );
    if smoke {
        format!("smoke-{stamp}")
    } else {
        stamp
    }
}

pub fn now_rfc3339() -> String {
    OffsetDateTime::now_utc().format(&Rfc3339).expect("rfc3339")
}

/// Запись через временный файл рядом и переименование (Global Constraints: записи атомарны).
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut name = path
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    name.push(".tmp");
    let tmp = path.with_file_name(name);
    {
        let mut f = File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)
}

/// Тег interval log не может содержать пробел, запятую и перевод строки.
pub fn hist_tag(operation: &str) -> String {
    operation
        .replace('%', "%25")
        .replace(' ', "%20")
        .replace(',', "%2C")
        .replace('\n', "%0A")
        .replace('\r', "%0D")
}

pub fn hist_untag(tag: &str) -> String {
    tag.replace("%0D", "\r")
        .replace("%0A", "\n")
        .replace("%2C", ",")
        .replace("%20", " ")
        .replace("%25", "%")
}

/// `hist.bin`: interval log `hdrhistogram`, по строке на операцию и на сценарий за весь прогон
/// (решение плана 7); тег сценария — `scenario:<id>`.
pub fn hist_log(totals: &Totals) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let mut serializer = V2Serializer::new();
    {
        let mut log = IntervalLogWriterBuilder::new()
            .add_comment(
                "glafira-load hist.bin: tag = operation id or scenario:<id>, space encoded as %20",
            )
            .begin_log_with(&mut out, &mut serializer)
            .map_err(|e| format!("hist.bin: {e}"))?;
        let duration = Duration::from_secs(totals.seconds.max(1));
        for (operation, op) in &totals.ops {
            let tag = hist_tag(operation);
            log.write_histogram(&op.hist, Duration::ZERO, duration, Tag::new(&tag))
                .map_err(|e| format!("hist.bin `{operation}`: {e:?}"))?;
        }
        for (scenario, sc) in &totals.scenarios {
            let tag = hist_tag(&format!("{SCENARIO_TAG}{scenario}"));
            log.write_histogram(&sc.hist, Duration::ZERO, duration, Tag::new(&tag))
                .map_err(|e| format!("hist.bin scenario `{scenario}`: {e:?}"))?;
        }
    }
    Ok(out)
}

/// Обратное чтение `hist.bin`: гистограммы по тегам (строки одного тега складываются), `n` — число
/// замеров в гистограмме, `per_second_max` — наибольшее число замеров в одной строке, `seconds` —
/// наибольший конец интервала. Ошибки по кодам, `dropped` и байты в interval log не хранятся и остаются
/// нулями — их контроллер берёт из суммы тиков. Строки без тега пропускаются; испорченная гистограмма — ошибка.
pub fn totals_from_hist_log(bytes: &[u8]) -> Result<Totals, String> {
    let mut totals = Totals {
        ops: BTreeMap::new(),
        scenarios: BTreeMap::new(),
        seconds: 0,
    };
    for (no, entry) in IntervalLogIterator::new(bytes).enumerate() {
        let entry = entry.map_err(|e| format!("hist.bin entry {}: {e:?}", no + 1))?;
        let LogEntry::Interval(interval) = entry else {
            continue;
        };
        let Some(tag) = interval.tag().map(|t| hist_untag(t.as_str())) else {
            continue;
        };
        let raw = base64::engine::general_purpose::STANDARD
            .decode(interval.encoded_histogram())
            .map_err(|e| format!("hist.bin `{tag}`: base64: {e}"))?;
        let hist: Histogram<u64> = Deserializer::new()
            .deserialize(&mut &raw[..])
            .map_err(|e| format!("hist.bin `{tag}`: histogram: {e}"))?;
        let end = interval.start_timestamp() + interval.duration();
        totals.seconds = totals.seconds.max(end.as_secs_f64().ceil() as u64);
        let count = hist.len();
        if let Some(id) = tag.strip_prefix(SCENARIO_TAG) {
            let e = totals
                .scenarios
                .entry(id.to_string())
                .or_insert_with(|| ScTotals {
                    hist: new_histogram(),
                    ..Default::default()
                });
            e.hist
                .add(&hist)
                .map_err(|e| format!("hist.bin `{tag}`: merge: {e}"))?;
            e.n += count;
            e.per_second_max = e.per_second_max.max(count);
        } else {
            let e = totals.ops.entry(tag.clone()).or_insert_with(|| OpTotals {
                hist: new_histogram(),
                ..Default::default()
            });
            e.hist
                .add(&hist)
                .map_err(|e| format!("hist.bin `{tag}`: merge: {e}"))?;
            e.n += count;
            e.per_second_max = e.per_second_max.max(count);
        }
    }
    Ok(totals)
}

/// Писатель папки прогона. `metrics.jsonl` и `errors.jsonl` открываются при первой записи:
/// смок их не создаёт (решение плана 5).
pub struct RunWriter {
    dir: PathBuf,
    meta: RunMeta,
    metrics: Option<BufWriter<File>>,
    errors: Option<BufWriter<File>>,
    phases: Vec<PhaseEvent>,
    /// Вторая защита предела примеров (первая — у агента): не больше пяти на код и операцию.
    samples: SampleCap,
}

impl RunWriter {
    /// Создаёт папку прогона (родители — по необходимости) и пишет `run.yaml`. Папка уже есть —
    /// ошибка `AlreadyExists`: два прогона, начатых в одну секунду, не пишут в одну папку.
    pub fn create(dir: &Path, meta: &RunMeta) -> io::Result<Self> {
        if let Some(parent) = dir.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(parent)?;
        }
        fs::create_dir(dir).map_err(|e| {
            if e.kind() == io::ErrorKind::AlreadyExists {
                io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!("run folder {} already exists", dir.display()),
                )
            } else {
                io::Error::new(e.kind(), format!("{}: {e}", dir.display()))
            }
        })?;
        let w = Self {
            dir: dir.to_path_buf(),
            meta: meta.clone(),
            metrics: None,
            errors: None,
            phases: Vec::new(),
            samples: SampleCap::new(),
        };
        w.write_run()?;
        Ok(w)
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn meta(&self) -> &RunMeta {
        &self.meta
    }

    fn write_run(&self) -> io::Result<()> {
        write_atomic(
            &self.dir.join(RUN_FILE),
            glafira_load_model::to_yaml(&self.meta).as_bytes(),
        )
    }

    fn append(
        file: &mut Option<BufWriter<File>>,
        dir: &Path,
        name: &str,
        line: &str,
    ) -> io::Result<()> {
        if file.is_none() {
            *file = Some(BufWriter::new(
                OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(dir.join(name))?,
            ));
        }
        let f = file.as_mut().expect("opened above");
        f.write_all(line.as_bytes())?;
        f.write_all(b"\n")
    }

    pub fn tick(&mut self, t: &Tick) -> io::Result<()> {
        for line in tick_lines(t) {
            Self::append(
                &mut self.metrics,
                &self.dir,
                METRICS_FILE,
                &serde_json::to_string(&line)?,
            )?;
        }
        if let Some(f) = self.metrics.as_mut() {
            f.flush()?;
        }
        Ok(())
    }

    pub fn phase(&mut self, p: &PhaseEvent) -> io::Result<()> {
        self.phases.push(p.clone());
        let file = PhasesFile {
            phases: self.phases.clone(),
        };
        write_atomic(
            &self.dir.join(PHASES_FILE),
            glafira_load_model::to_yaml(&file).as_bytes(),
        )
    }

    /// Дописывает пример в `errors.jsonl`; сверх `MAX_SAMPLES_PER_CODE_OP` на код и операцию
    /// пример молча отбрасывается.
    pub fn error(&mut self, e: &ErrorSample) -> io::Result<()> {
        if !self.samples.allow(&e.code, &e.operation) {
            return Ok(());
        }
        Self::append(
            &mut self.errors,
            &self.dir,
            ERRORS_FILE,
            &serde_json::to_string(e)?,
        )?;
        if let Some(f) = self.errors.as_mut() {
            f.flush()?;
        }
        Ok(())
    }

    pub fn smoke(&self, report: &SmokeReport) -> io::Result<()> {
        write_atomic(
            &self.dir.join(SMOKE_FILE),
            glafira_load_model::to_yaml(report).as_bytes(),
        )
    }

    /// `summary.yaml` (операции и сценарии по id) и `run.yaml` с `ended_at` и `status`.
    pub fn finish(&mut self, summary: &AgentSummary, status: RunStatus) -> io::Result<()> {
        if let Some(f) = self.metrics.as_mut() {
            f.flush()?;
        }
        if let Some(f) = self.errors.as_mut() {
            f.flush()?;
        }
        write_atomic(
            &self.dir.join(SUMMARY_FILE),
            glafira_load_model::to_yaml(&sorted_summary(summary)).as_bytes(),
        )?;
        self.meta.ended_at = Some(now_rfc3339());
        self.meta.status = Some(status);
        self.write_run()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use hdrhistogram::Histogram;
    use hdrhistogram::serialization::interval_log::{IntervalLogIterator, LogEntry};
    use time::UtcOffset;

    use super::*;
    use crate::agent_api::{OpSummary, OpTick, ScenarioTick};
    use crate::metrics::{OpTotals, Totals};

    pub(crate) fn meta() -> RunMeta {
        RunMeta {
            run_id: "r1".into(),
            suite: "orders".into(),
            profile: "peak".into(),
            env: "test".into(),
            mode: Mode::Run,
            agents: vec![RunAgent {
                id: "local".into(),
                url: None,
                weight: 1,
                version: Some("0.2.0".into()),
            }],
            project_commit: Some("abc123".into()),
            services: BTreeMap::from([("orders".to_string(), "1.2.0".to_string())]),
            started_at: "2026-11-02T14:30:00Z".into(),
            ended_at: None,
            status: None,
        }
    }

    fn op_summary(id: &str) -> OpSummary {
        OpSummary {
            id: id.into(),
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
            target: None,
            achieved_pct: None,
        }
    }

    #[test]
    fn run_dir_name_is_a_utc_stamp() {
        let t = OffsetDateTime::from_unix_timestamp(1_793_629_800).unwrap(); // 2026-11-02T14:30:00Z
        assert_eq!(run_dir_name(t, false), "2026-11-02T14-30-00");
        assert_eq!(run_dir_name(t, true), "smoke-2026-11-02T14-30-00");
        let local = t.to_offset(UtcOffset::from_hms(3, 0, 0).unwrap());
        assert_eq!(
            run_dir_name(local, false),
            "2026-11-02T14-30-00",
            "всегда UTC"
        );
        assert!(now_rfc3339().ends_with('Z'));
    }

    #[test]
    fn tick_lines_cover_ops_and_scenarios() {
        let tick = Tick {
            t: 12,
            ops: vec![OpTick {
                id: "kafka_consume:orders.created".into(),
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
                lag: Some(12),
                hist: None,
            }],
            scenarios: vec![ScenarioTick {
                id: "order-flow".into(),
                n: 100,
                err: BTreeMap::new(),
                dropped: 2,
                p50: 50.0,
                p95: 90.0,
                p99: 95.0,
                max: 99.0,
                hist: None,
            }],
        };
        let lines: Vec<String> = tick_lines(&tick)
            .iter()
            .map(|l| serde_json::to_string(l).unwrap())
            .collect();
        assert_eq!(
            lines[0],
            r#"{"t":12,"op":"kafka_consume:orders.created","n":480,"err":{"HTTP_5xx":3},"dropped":0,"p50":41.0,"p90":80.0,"p95":97.0,"p99":180.0,"max":412.0,"bytes_in":120400,"bytes_out":51200,"lag":12}"#
        );
        assert_eq!(
            lines[1],
            r#"{"t":12,"scenario":"order-flow","n":100,"err":{},"dropped":2,"p50":50.0,"p95":90.0,"p99":95.0,"max":99.0}"#
        );
        let back: MetricLine = serde_json::from_str(&lines[0]).unwrap();
        assert_eq!(back.op.as_deref(), Some("kafka_consume:orders.created"));
    }

    #[test]
    fn writer_creates_files_and_finishes_run_yaml() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("runs").join("2026-11-02T14-30-00");
        let mut w = RunWriter::create(&dir, &meta()).unwrap();
        assert_eq!(w.dir(), dir.as_path());
        let run = std::fs::read_to_string(dir.join(RUN_FILE)).unwrap();
        assert!(
            run.starts_with("run_id: r1\nsuite: orders\nprofile: peak\nenv: test\n"),
            "{run}"
        );
        assert!(
            !run.contains("ended_at") && !run.contains("status:"),
            "{run}"
        );
        assert!(
            !dir.join(METRICS_FILE).exists(),
            "metrics.jsonl появляется с первым тиком (смок его не пишет)"
        );

        let tick = Tick {
            t: 1,
            ops: vec![OpTick {
                id: "a".into(),
                n: 1,
                err: BTreeMap::new(),
                dropped: 0,
                p50: 1.0,
                p90: 1.0,
                p95: 1.0,
                p99: 1.0,
                max: 1.0,
                bytes_in: 0,
                bytes_out: 0,
                lag: None,
                hist: None,
            }],
            scenarios: vec![],
        };
        w.tick(&tick).unwrap();
        w.tick(&tick).unwrap();
        w.phase(&PhaseEvent {
            t: 0,
            index: 0,
            pct: 0.0,
        })
        .unwrap();
        w.phase(&PhaseEvent {
            t: 60,
            index: 1,
            pct: 100.0,
        })
        .unwrap();
        w.error(&ErrorSample {
            t: 3,
            code: "HTTP_5xx".into(),
            operation: "a".into(),
            scenario: "s".into(),
            step: "create".into(),
            agent: "local".into(),
            message: "HTTP 503".into(),
            request: Some("POST /v1/orders".into()),
            response: None,
        })
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join(METRICS_FILE))
                .unwrap()
                .lines()
                .count(),
            2
        );
        let phases: PhasesFile =
            glafira_load_model::from_yaml(&std::fs::read_to_string(dir.join(PHASES_FILE)).unwrap())
                .unwrap();
        assert_eq!(phases.phases.len(), 2);
        let errors = std::fs::read_to_string(dir.join(ERRORS_FILE)).unwrap();
        assert_eq!(errors.lines().count(), 1);
        let back: ErrorSample = serde_json::from_str(errors.lines().next().unwrap()).unwrap();
        assert_eq!(back.code, "HTTP_5xx");

        let summary = AgentSummary {
            status: RunStatus::Completed,
            ops: vec![op_summary("b"), op_summary("a")],
            scenarios: vec![],
            seconds: 2,
        };
        w.finish(&summary, RunStatus::Completed).unwrap();
        let run = std::fs::read_to_string(dir.join(RUN_FILE)).unwrap();
        assert!(
            run.contains("\nended_at: ") && run.contains("\nstatus: completed\n"),
            "{run}"
        );
        let written: RunMeta = glafira_load_model::from_yaml(&run).unwrap();
        assert_eq!(
            (written.status, written.run_id.as_str()),
            (Some(RunStatus::Completed), "r1")
        );
        let text = std::fs::read_to_string(dir.join(SUMMARY_FILE)).unwrap();
        assert!(
            text.find("id: a").unwrap() < text.find("id: b").unwrap(),
            "операции отсортированы: {text}"
        );
        let back: AgentSummary = glafira_load_model::from_yaml(&text).unwrap();
        assert_eq!(back.ops.len(), 2);
        assert!(!dir.join(SMOKE_FILE).exists());
    }

    fn error_sample(code: &str, operation: &str) -> ErrorSample {
        ErrorSample {
            t: 1,
            code: code.into(),
            operation: operation.into(),
            scenario: "s".into(),
            step: "create".into(),
            agent: "local".into(),
            message: "boom".into(),
            request: None,
            response: None,
        }
    }

    #[test]
    fn writer_keeps_five_error_samples_per_code_and_operation() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("run");
        let mut w = RunWriter::create(&dir, &meta()).unwrap();
        for _ in 0..20 {
            w.error(&error_sample("HTTP_5xx", "a")).unwrap();
            w.error(&error_sample("TIMEOUT", "a")).unwrap();
        }
        let text = std::fs::read_to_string(dir.join(ERRORS_FILE)).unwrap();
        let codes: Vec<String> = text
            .lines()
            .map(|l| serde_json::from_str::<ErrorSample>(l).unwrap().code)
            .collect();
        assert_eq!(codes.len(), 2 * crate::errors::MAX_SAMPLES_PER_CODE_OP);
        assert_eq!(codes.iter().filter(|c| *c == "HTTP_5xx").count(), 5);
        assert_eq!(codes.iter().filter(|c| *c == "TIMEOUT").count(), 5);
    }

    #[test]
    fn writer_refuses_an_existing_run_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("runs").join("2026-11-02T14-30-00");
        RunWriter::create(&dir, &meta()).unwrap();
        let err = RunWriter::create(&dir, &meta())
            .err()
            .expect("folder exists");
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert!(err.to_string().contains("already exists"), "{err}");
        assert!(
            err.to_string().contains("2026-11-02T14-30-00"),
            "в тексте путь папки: {err}"
        );
    }

    #[test]
    fn atomic_write_replaces_existing_file() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("x.yaml");
        write_atomic(&path, b"one").unwrap();
        write_atomic(&path, b"two").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "two");
        assert_eq!(
            std::fs::read_dir(tmp.path()).unwrap().count(),
            1,
            "временный файл переименован"
        );
    }

    #[test]
    fn hist_tag_round_trips_operation_ids() {
        for id in [
            "http:orders:POST /v1/orders",
            "kafka_consume:a,b",
            "x%20y",
            "plain",
        ] {
            let tag = hist_tag(id);
            assert!(!tag.contains(' ') && !tag.contains(','), "{tag}");
            assert_eq!(hist_untag(&tag), id);
        }
    }

    #[test]
    fn hist_log_has_one_tagged_line_per_operation_and_scenario() {
        let mut hist = Histogram::<u64>::new_with_bounds(1, 60_000_000, 3).unwrap();
        for v in [1_000, 2_000, 3_000] {
            hist.record(v).unwrap();
        }
        let op = |h: &Histogram<u64>| OpTotals {
            hist: h.clone(),
            n: 3,
            err: BTreeMap::new(),
            dropped: 0,
            bytes_in: 0,
            bytes_out: 0,
            per_second_max: 3,
        };
        let totals = Totals {
            ops: BTreeMap::from([
                ("http:orders:POST /v1/orders".to_string(), op(&hist)),
                ("kafka_produce:orders.paid".to_string(), op(&hist)),
            ]),
            scenarios: BTreeMap::from([(
                "order-flow".to_string(),
                ScTotals {
                    hist: hist.clone(),
                    n: 3,
                    err: BTreeMap::new(),
                    dropped: 1,
                    per_second_max: 3,
                },
            )]),
            seconds: 5,
        };
        let log = hist_log(&totals).unwrap();
        let tags: Vec<String> = IntervalLogIterator::new(&log)
            .filter_map(|e| match e {
                Ok(LogEntry::Interval(h)) => {
                    Some(hist_untag(h.tag().map(|t| t.as_str()).unwrap_or("")))
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            tags,
            [
                "http:orders:POST /v1/orders",
                "kafka_produce:orders.paid",
                "scenario:order-flow"
            ]
        );

        // Обратное чтение: гистограммы и число замеров восстанавливаются, секунды — из длительности.
        let back = totals_from_hist_log(&log).unwrap();
        assert_eq!(
            back.ops.keys().cloned().collect::<Vec<_>>(),
            ["http:orders:POST /v1/orders", "kafka_produce:orders.paid"]
        );
        let post = &back.ops["http:orders:POST /v1/orders"];
        assert_eq!(
            (post.n, post.hist.len(), post.hist.max()),
            (3, 3, hist.max())
        );
        assert_eq!(
            (
                back.scenarios["order-flow"].n,
                back.scenarios["order-flow"].hist.len()
            ),
            (3, 3)
        );
        assert_eq!(back.seconds, 5);
        // Две записи одного тега складываются (так контроллер сливает агентов, писавших по строке на секунду).
        let tagged_lines: Vec<u8> = log
            .split(|b| *b == b'\n')
            .filter(|l| l.starts_with(b"Tag="))
            .flat_map(|l| l.iter().copied().chain(std::iter::once(b'\n')))
            .collect();
        let mut twice = log.clone();
        twice.extend_from_slice(&tagged_lines);
        let merged = totals_from_hist_log(&twice).unwrap();
        assert_eq!(merged.ops["http:orders:POST /v1/orders"].hist.len(), 6);
        assert!(
            totals_from_hist_log(b"garbage,1,2\n").is_err(),
            "строка не interval log"
        );
        assert!(
            totals_from_hist_log(b"Tag=x,0.000,1.000,0.000,!!!\n").is_err(),
            "испорченная гистограмма"
        );
        assert!(
            totals_from_hist_log(b"#only a comment\n")
                .unwrap()
                .ops
                .is_empty()
        );
    }
}
