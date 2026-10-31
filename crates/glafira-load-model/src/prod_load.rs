//! Цифры нагрузки ПРОМ `load/prod-load.yaml` и предпросмотр импорта шаблона (спека L1 §7.2–§7.3,
//! решение плана 3: у всех операций `avg` и `peak`).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::registry::OpKind;

pub const PROD_LOAD_FILE: &str = "load/prod-load.yaml";
pub const IMPORTS_DIR: &str = "load/imports";
pub const FORMAT: u32 = 1;

/// Лист «Параметры» шаблона.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SourceInfo {
    #[serde(default)]
    pub stand: String,
    #[serde(default)]
    pub period_from: String,
    #[serde(default)]
    pub period_to: String,
    #[serde(default)]
    pub system: String,
    #[serde(default)]
    pub filled_by: String,
    /// RFC 3339, UTC.
    #[serde(default)]
    pub imported_at: String,
    /// Копия шаблона относительно проекта: `load/imports/<метка>.xlsx`.
    #[serde(default)]
    pub file: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Figures {
    /// Среднее RPS (сообщений/с у `kafka_consume`).
    pub avg: f64,
    pub peak: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_pct: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub req_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resp_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub p95_ms: Option<f64>,
    /// Только `kafka_consume`: допустимый лаг, с.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lag_max_s: Option<f64>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub comment: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProdLoad {
    pub format: u32,
    #[serde(default)]
    pub source: SourceInfo,
    #[serde(default)]
    pub operations: BTreeMap<String, Figures>,
}

impl Default for ProdLoad {
    fn default() -> Self {
        Self {
            format: FORMAT,
            source: SourceInfo::default(),
            operations: BTreeMap::new(),
        }
    }
}

/// Ячейка из шаблона: пустая, число или текст, который числом не является.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Cell {
    Empty,
    Number { value: f64 },
    Text { value: String },
}

impl Cell {
    pub fn is_empty(&self) -> bool {
        matches!(self, Cell::Empty)
    }
}

/// Строка листа «Операции» как её прочитал `glafira-load-xlsx`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TemplateRow {
    /// Номер строки в листе, с 1.
    pub row: u32,
    pub id: String,
    pub avg: Cell,
    pub peak: Cell,
    pub error_pct: Cell,
    pub req_bytes: Cell,
    pub resp_bytes: Cell,
    /// p95 (мс) у всех, кроме `kafka_consume`, где это допустимый лаг (с).
    pub p95_ms: Cell,
    #[serde(default)]
    pub comment: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InvalidReason {
    NotANumber,
    Negative,
    NotAnInteger,
    PercentOutOfRange,
    PeakBelowAvg,
    AvgMissing,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InvalidCell {
    pub row: u32,
    pub id: String,
    /// `avg`, `peak`, `error_pct`, `req_bytes`, `resp_bytes`, `p95_ms`.
    pub column: String,
    pub value: String,
    pub reason: InvalidReason,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FiguresDiff {
    pub id: String,
    pub old: Option<Figures>,
    pub new: Figures,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ImportPreview {
    pub filled: Vec<String>,
    pub empty: Vec<String>,
    pub unknown_ids: Vec<String>,
    pub invalid: Vec<InvalidCell>,
    pub diff: Vec<FiguresDiff>,
    pub source: SourceInfo,
    /// Будущий `prod-load.yaml`; `None`, пока есть `invalid`.
    pub result: Option<ProdLoad>,
}

fn number(
    cell: &Cell,
    column: &str,
    row: &TemplateRow,
    invalid: &mut Vec<InvalidCell>,
) -> Option<f64> {
    let bad = |value: String, reason| InvalidCell {
        row: row.row,
        id: row.id.clone(),
        column: column.into(),
        value,
        reason,
    };
    match cell {
        Cell::Empty => None,
        Cell::Text { value } => {
            invalid.push(bad(value.clone(), InvalidReason::NotANumber));
            None
        }
        Cell::Number { value } if *value < 0.0 => {
            invalid.push(bad(value.to_string(), InvalidReason::Negative));
            None
        }
        Cell::Number { value } => Some(*value),
    }
}

fn integer(
    cell: &Cell,
    column: &str,
    row: &TemplateRow,
    invalid: &mut Vec<InvalidCell>,
) -> Option<u64> {
    let v = number(cell, column, row, invalid)?;
    if v.fract() != 0.0 {
        invalid.push(InvalidCell {
            row: row.row,
            id: row.id.clone(),
            column: column.into(),
            value: v.to_string(),
            reason: InvalidReason::NotAnInteger,
        });
        return None;
    }
    Some(v as u64)
}

/// Предпросмотр импорта. Пустые строки пропускаются (прежние цифры остаются), неизвестные `id`
/// пропускаются с предупреждением, строки с ошибками блокируют результат.
pub fn preview_import(
    rows: &[TemplateRow],
    source: SourceInfo,
    known: &BTreeMap<String, OpKind>,
    current: Option<&ProdLoad>,
) -> ImportPreview {
    let mut p = ImportPreview {
        filled: vec![],
        empty: vec![],
        unknown_ids: vec![],
        invalid: vec![],
        diff: vec![],
        source: source.clone(),
        result: None,
    };
    let mut result = current.cloned().unwrap_or_default();
    result.source = source;
    for row in rows {
        let Some(kind) = known.get(&row.id) else {
            p.unknown_ids.push(row.id.clone());
            continue;
        };
        if [
            &row.avg,
            &row.peak,
            &row.error_pct,
            &row.req_bytes,
            &row.resp_bytes,
            &row.p95_ms,
        ]
        .iter()
        .all(|c| c.is_empty())
        {
            p.empty.push(row.id.clone());
            continue;
        }
        let before = p.invalid.len();
        let avg = number(&row.avg, "avg", row, &mut p.invalid);
        let peak = number(&row.peak, "peak", row, &mut p.invalid);
        let error_pct = number(&row.error_pct, "error_pct", row, &mut p.invalid);
        if let Some(e) = error_pct
            && e > 100.0
        {
            p.invalid.push(InvalidCell {
                row: row.row,
                id: row.id.clone(),
                column: "error_pct".into(),
                value: e.to_string(),
                reason: InvalidReason::PercentOutOfRange,
            });
        }
        let req_bytes = integer(&row.req_bytes, "req_bytes", row, &mut p.invalid);
        let resp_bytes = integer(&row.resp_bytes, "resp_bytes", row, &mut p.invalid);
        let p95 = number(&row.p95_ms, "p95_ms", row, &mut p.invalid);
        let Some(avg) = avg else {
            if row.avg.is_empty() {
                p.invalid.push(InvalidCell {
                    row: row.row,
                    id: row.id.clone(),
                    column: "avg".into(),
                    value: String::new(),
                    reason: InvalidReason::AvgMissing,
                });
            }
            continue;
        };
        let peak = peak.unwrap_or(avg);
        if peak < avg {
            p.invalid.push(InvalidCell {
                row: row.row,
                id: row.id.clone(),
                column: "peak".into(),
                value: peak.to_string(),
                reason: InvalidReason::PeakBelowAvg,
            });
        }
        if p.invalid.len() != before {
            continue;
        }
        let consume = *kind == OpKind::KafkaConsume;
        let figures = Figures {
            avg,
            peak,
            error_pct,
            req_bytes,
            resp_bytes,
            p95_ms: if consume { None } else { p95 },
            lag_max_s: if consume { p95 } else { None },
            comment: row.comment.trim().to_string(),
        };
        let old = current.and_then(|c| c.operations.get(&row.id)).cloned();
        if old.as_ref() != Some(&figures) {
            p.diff.push(FiguresDiff {
                id: row.id.clone(),
                old,
                new: figures.clone(),
            });
        }
        result.operations.insert(row.id.clone(), figures);
        p.filled.push(row.id.clone());
    }
    if p.invalid.is_empty() {
        p.result = Some(result);
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(n: u32, id: &str, avg: Cell, peak: Cell) -> TemplateRow {
        TemplateRow {
            row: n,
            id: id.into(),
            avg,
            peak,
            error_pct: Cell::Empty,
            req_bytes: Cell::Empty,
            resp_bytes: Cell::Empty,
            p95_ms: Cell::Empty,
            comment: String::new(),
        }
    }
    fn num(v: f64) -> Cell {
        Cell::Number { value: v }
    }
    fn known() -> BTreeMap<String, OpKind> {
        [
            ("http:o:POST /a", OpKind::Http),
            ("kafka_consume:t", OpKind::KafkaConsume),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect()
    }

    #[test]
    fn classifies_rows_and_keeps_previous_figures_for_empty_ones() {
        let mut current = ProdLoad::default();
        current.operations.insert(
            "kafka_consume:t".into(),
            Figures {
                avg: 5.0,
                peak: 6.0,
                ..Default::default()
            },
        );
        let mut consume = row(3, "kafka_consume:t", Cell::Empty, Cell::Empty);
        consume.p95_ms = Cell::Empty;
        let rows = vec![
            row(2, "http:o:POST /a", num(120.0), num(340.0)),
            consume,
            row(4, "http:o:GET /old", num(1.0), Cell::Empty),
        ];
        let p = preview_import(
            &rows,
            SourceInfo {
                stand: "prod".into(),
                ..Default::default()
            },
            &known(),
            Some(&current),
        );
        assert_eq!(p.filled, vec!["http:o:POST /a"]);
        assert_eq!(p.empty, vec!["kafka_consume:t"]);
        assert_eq!(p.unknown_ids, vec!["http:o:GET /old"]);
        assert!(p.invalid.is_empty());
        assert_eq!(p.diff.len(), 1);
        assert_eq!(p.diff[0].old, None);
        let r = p.result.unwrap();
        assert_eq!(
            r.operations["kafka_consume:t"].avg, 5.0,
            "пустая строка не стирает прежние цифры"
        );
        assert_eq!(r.operations["http:o:POST /a"].peak, 340.0);
        assert_eq!(r.source.stand, "prod");
    }

    #[test]
    fn invalid_cells_block_the_result() {
        let mut r1 = row(
            2,
            "http:o:POST /a",
            Cell::Text {
                value: "many".into(),
            },
            num(1.0),
        );
        r1.error_pct = num(150.0);
        r1.req_bytes = num(1.5);
        let r2 = row(3, "kafka_consume:t", num(10.0), num(5.0));
        let mut r3 = row(4, "http:o:POST /a", Cell::Empty, num(5.0));
        r3.row = 4;
        let p = preview_import(&[r1, r2, r3], SourceInfo::default(), &known(), None);
        let reasons: Vec<(u32, &str, InvalidReason)> = p
            .invalid
            .iter()
            .map(|i| (i.row, i.column.as_str(), i.reason))
            .collect();
        assert_eq!(
            reasons,
            vec![
                (2, "avg", InvalidReason::NotANumber),
                (2, "error_pct", InvalidReason::PercentOutOfRange),
                (2, "req_bytes", InvalidReason::NotAnInteger),
                (3, "peak", InvalidReason::PeakBelowAvg),
                (4, "avg", InvalidReason::AvgMissing),
            ]
        );
        assert!(p.result.is_none());
        assert!(p.filled.is_empty());
    }

    #[test]
    fn consume_rows_put_the_last_column_into_lag() {
        let mut r = row(2, "kafka_consume:t", num(10.0), Cell::Empty);
        r.p95_ms = num(5.0);
        let p = preview_import(&[r], SourceInfo::default(), &known(), None);
        let f = &p.result.unwrap().operations["kafka_consume:t"];
        assert_eq!((f.peak, f.lag_max_s, f.p95_ms), (10.0, Some(5.0), None));
    }

    #[test]
    fn yaml_round_trip() {
        let mut pl = ProdLoad::default();
        pl.operations.insert(
            "http:o:POST /a".into(),
            Figures {
                avg: 120.0,
                peak: 340.0,
                error_pct: Some(0.2),
                req_bytes: Some(850),
                resp_bytes: None,
                p95_ms: Some(180.0),
                lag_max_s: None,
                comment: "x".into(),
            },
        );
        let text = crate::to_yaml(&pl);
        assert!(!text.contains("resp_bytes"), "{text}");
        assert_eq!(crate::from_yaml::<ProdLoad>(&text).unwrap(), pl);
    }
}
