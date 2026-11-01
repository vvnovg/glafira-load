//! Консольный вывод: без цветов, ширина колонок по содержимому (спека §5.4).

use glafira_load_engine::agent_api::{AgentSummary, RunStatus, Tick};
use glafira_load_engine::summary::SmokeReport;
use glafira_load_model::suite::check::{Finding, Level};

/// Таблица: заголовки и строки, колонки разделены двумя пробелами, ширина — по самой длинной ячейке.
pub fn table(headers: &[&str], rows: &[Vec<String>]) -> String {
    let cols = headers.len();
    let mut width: Vec<usize> = headers.iter().map(|h| h.chars().count()).collect();
    for row in rows {
        for (i, cell) in row.iter().enumerate().take(cols) {
            width[i] = width[i].max(cell.chars().count());
        }
    }
    let line = |cells: &[String]| -> String {
        let mut s = String::new();
        for (i, cell) in cells.iter().enumerate().take(cols) {
            if i > 0 {
                s.push_str("  ");
            }
            s.push_str(cell);
            if i + 1 < cols {
                s.extend(std::iter::repeat_n(' ', width[i] - cell.chars().count()));
            }
        }
        s.push('\n');
        s
    };
    let header: Vec<String> = headers.iter().map(|h| h.to_string()).collect();
    let mut out = line(&header);
    for row in rows {
        out.push_str(&line(row));
    }
    out
}

/// `error|warning  file:line  message`; без строки — только файл.
pub fn finding_line(f: &Finding) -> String {
    let level = match f.level {
        Level::Error => "error",
        Level::Warning => "warning",
    };
    let place = match f.line {
        Some(line) => format!("{}:{line}", f.file),
        None => f.file.clone(),
    };
    format!("{level:<7}  {place}  {}", f.message)
}

/// Отчёт `check`: строки находок и итог `N error(s), M warning(s)`.
pub fn check_report(findings: &[Finding]) -> String {
    let mut out = String::new();
    for f in findings {
        out.push_str(&finding_line(f));
        out.push('\n');
    }
    let errors = findings
        .iter()
        .filter(|f| matches!(f.level, Level::Error))
        .count();
    out.push_str(&format!(
        "{errors} error(s), {} warning(s)\n",
        findings.len() - errors
    ));
    out
}

/// Таблица смока: сценарий, шаг, операция, результат (`ok` либо код и причина; пропущенные — `SKIPPED`).
pub fn smoke_table(report: &SmokeReport) -> String {
    let rows: Vec<Vec<String>> = report
        .scenarios
        .iter()
        .flat_map(|s| {
            s.steps.iter().map(move |st| {
                let result = if st.ok {
                    "ok".to_string()
                } else {
                    match (&st.code, &st.message) {
                        (Some(code), Some(m)) => format!("{code} {m}"),
                        (Some(code), None) => code.clone(),
                        (None, Some(m)) => m.clone(),
                        (None, None) => "failed".to_string(),
                    }
                };
                vec![s.id.clone(), st.id.clone(), st.operation.clone(), result]
            })
        })
        .collect();
    let mut out = table(&["scenario", "step", "operation", "result"], &rows);
    out.push_str(&format!("{} step(s) failed\n", report.failed_steps()));
    out
}

/// Строка в секунду: `t=12s rps=480 p95=97ms err=3 dropped=0`; p95 — худший по операциям.
pub fn tick_line(t: &Tick) -> String {
    let rps: u64 = t.ops.iter().map(|o| o.n).sum();
    let p95 = t.ops.iter().map(|o| o.p95).fold(0.0_f64, f64::max);
    let err: u64 = t.ops.iter().map(|o| o.err.values().sum::<u64>()).sum();
    let dropped: u64 = t.ops.iter().map(|o| o.dropped).sum();
    format!(
        "t={}s rps={rps} p95={}ms err={err} dropped={dropped}",
        t.t,
        p95.round() as u64
    )
}

/// Итог по операциям: id, n, rps avg, p95, p99, err %, target, achieved %.
pub fn summary_table(s: &AgentSummary) -> String {
    let ms = |v: f64| format!("{}", v.round() as u64);
    let rows: Vec<Vec<String>> = s
        .ops
        .iter()
        .map(|o| {
            vec![
                o.id.clone(),
                o.n.to_string(),
                format!("{:.1}", o.rps_avg),
                ms(o.p95),
                ms(o.p99),
                format!("{:.2}", o.err_pct),
                o.target
                    .map(|t| format!("{t:.1}"))
                    .unwrap_or_else(|| "-".into()),
                o.achieved_pct
                    .map(|p| format!("{p:.0}"))
                    .unwrap_or_else(|| "-".into()),
            ]
        })
        .collect();
    table(
        &[
            "operation",
            "n",
            "rps avg",
            "p95 ms",
            "p99 ms",
            "err %",
            "target",
            "achieved %",
        ],
        &rows,
    )
}

pub fn status_label(s: &RunStatus) -> &'static str {
    match s {
        RunStatus::Completed => "completed",
        RunStatus::Stopped => "stopped",
        RunStatus::Incomplete => "incomplete",
        RunStatus::Failed => "failed",
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use glafira_load_engine::agent_api::{OpSummary, OpTick};
    use glafira_load_engine::summary::{SmokeScenario, SmokeStep};

    use super::*;

    #[test]
    fn findings_and_summary_line() {
        let findings = vec![
            Finding {
                level: Level::Error,
                file: "suites/a/suite.yaml".into(),
                line: Some(3),
                message: "bad".into(),
            },
            Finding {
                level: Level::Warning,
                file: "suites/a/suite.yaml".into(),
                line: None,
                message: "meh".into(),
            },
        ];
        let text = check_report(&findings);
        assert_eq!(
            text,
            "error    suites/a/suite.yaml:3  bad\nwarning  suites/a/suite.yaml  meh\n1 error(s), 1 warning(s)\n"
        );
    }

    #[test]
    fn tick_line_sums_ops_and_takes_worst_p95() {
        let op = |id: &str, n: u64, p95: f64, err: u64| OpTick {
            id: id.into(),
            n,
            err: BTreeMap::from([("HTTP_5xx".to_string(), err)]),
            dropped: 0,
            p50: 1.0,
            p90: 2.0,
            p95,
            p99: 4.0,
            max: 5.0,
            bytes_in: 0,
            bytes_out: 0,
            lag: None,
            hist: None,
        };
        let t = Tick {
            t: 12,
            ops: vec![op("a", 300, 97.2, 3), op("b", 180, 40.0, 0)],
            scenarios: vec![],
        };
        assert_eq!(tick_line(&t), "t=12s rps=480 p95=97ms err=3 dropped=0");
    }

    #[test]
    fn tables_fit_the_content() {
        let s = AgentSummary {
            status: RunStatus::Completed,
            ops: vec![OpSummary {
                id: "http:orders:POST /v1/orders".into(),
                n: 1200,
                rps_avg: 20.0,
                rps_peak: 25.0,
                err: BTreeMap::new(),
                err_pct: 0.0,
                dropped: 0,
                p50: 10.0,
                p90: 20.0,
                p95: 30.4,
                p99: 50.0,
                max: 90.0,
                target: Some(20.0),
                achieved_pct: Some(100.0),
            }],
            scenarios: vec![],
            seconds: 60,
        };
        let text = summary_table(&s);
        assert!(text.starts_with("operation                    n     rps avg  p95 ms  p99 ms  err %  target  achieved %\n"));
        assert!(text.contains(
            "http:orders:POST /v1/orders  1200  20.0     30      50      0.00   20.0    100\n"
        ));
        let report = SmokeReport {
            scenarios: vec![SmokeScenario {
                id: "s".into(),
                steps: vec![
                    SmokeStep {
                        id: "create".into(),
                        operation: "http:o:POST /x".into(),
                        ok: false,
                        code: Some("HTTP_5xx".into()),
                        message: Some("500".into()),
                    },
                    SmokeStep {
                        id: "get".into(),
                        operation: "http:o:GET /x".into(),
                        ok: true,
                        code: None,
                        message: None,
                    },
                ],
            }],
        };
        assert_eq!(
            smoke_table(&report),
            "scenario  step    operation       result\ns         create  http:o:POST /x  HTTP_5xx 500\ns         get     http:o:GET /x   ok\n1 step(s) failed\n"
        );
        assert_eq!(status_label(&RunStatus::Stopped), "stopped");
    }
}
