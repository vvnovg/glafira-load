//! Метрики прогона на `hdrhistogram` (спека L2 §5.3): посекундные срезы, итоги, слияние и сводка.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use hdrhistogram::Histogram;
use hdrhistogram::serialization::{Deserializer, Serializer as _, V2DeflateSerializer};

use crate::agent_api::{
    AgentSummary, OpSummary, OpTick, RunStatus, ScenarioSummary, ScenarioTick, Tick,
};
use crate::errors::StepCode;

/// Нижняя граница гистограммы: 1 мкс.
pub const HIST_LOW_US: u64 = 1;
/// Верхняя граница гистограммы: 60 с.
pub const HIST_HIGH_US: u64 = 60_000_000;
/// Значащих цифр.
pub const HIST_SIGFIG: u8 = 3;

/// Гистограмма задержек в микросекундах с границами спеки.
pub fn new_histogram() -> Histogram<u64> {
    Histogram::new_with_bounds(HIST_LOW_US, HIST_HIGH_US, HIST_SIGFIG).expect("valid bounds")
}

/// Гистограмма для поля `hist` среза: `hdrhistogram` V2 со сжатием deflate, затем base64.
pub fn encode_hist(h: &Histogram<u64>) -> String {
    let mut buf = Vec::new();
    V2DeflateSerializer::new()
        .serialize(h, &mut buf)
        .expect("serializing a u64 histogram into memory does not fail");
    BASE64.encode(buf)
}

/// Обратное к [`encode_hist`]: base64 → `hdrhistogram::serialization::Deserializer`.
/// Ошибка — текстом: поле приходит от агента и может быть испорчено.
pub fn decode_hist(text: &str) -> Result<Histogram<u64>, String> {
    let bytes = BASE64
        .decode(text)
        .map_err(|e| format!("bad base64 in histogram: {e}"))?;
    Deserializer::new()
        .deserialize(&mut bytes.as_slice())
        .map_err(|e| format!("bad histogram: {e:?}"))
}

/// Гистограмма для среза: `None`, если за секунду не было замеров.
fn tick_hist(h: &Histogram<u64>) -> Option<String> {
    (!h.is_empty()).then(|| encode_hist(h))
}

/// Один измеренный шаг.
#[derive(Debug, Clone, PartialEq)]
pub struct Sample {
    pub operation: String,
    pub scenario: String,
    pub latency_us: u64,
    pub bytes_in: u64,
    pub bytes_out: u64,
    pub error: Option<StepCode>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OpTotals {
    pub hist: Histogram<u64>,
    pub n: u64,
    pub err: BTreeMap<String, u64>,
    pub dropped: u64,
    pub bytes_in: u64,
    pub bytes_out: u64,
    /// Наибольшее число запросов за одну секунду.
    pub per_second_max: u64,
}

impl Default for OpTotals {
    fn default() -> Self {
        Self {
            hist: new_histogram(),
            n: 0,
            err: BTreeMap::new(),
            dropped: 0,
            bytes_in: 0,
            bytes_out: 0,
            per_second_max: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScTotals {
    pub hist: Histogram<u64>,
    pub n: u64,
    pub err: BTreeMap<String, u64>,
    pub dropped: u64,
    pub per_second_max: u64,
}

impl Default for ScTotals {
    fn default() -> Self {
        Self {
            hist: new_histogram(),
            n: 0,
            err: BTreeMap::new(),
            dropped: 0,
            per_second_max: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Totals {
    pub ops: BTreeMap<String, OpTotals>,
    pub scenarios: BTreeMap<String, ScTotals>,
    pub seconds: u64,
}

/// Срез операции за текущую секунду.
#[derive(Debug, Clone)]
struct OpSecond {
    hist: Histogram<u64>,
    n: u64,
    err: BTreeMap<String, u64>,
    dropped: u64,
    bytes_in: u64,
    bytes_out: u64,
}

impl Default for OpSecond {
    fn default() -> Self {
        Self {
            hist: new_histogram(),
            n: 0,
            err: BTreeMap::new(),
            dropped: 0,
            bytes_in: 0,
            bytes_out: 0,
        }
    }
}

#[derive(Debug, Clone)]
struct ScSecond {
    hist: Histogram<u64>,
    n: u64,
    err: BTreeMap<String, u64>,
    dropped: u64,
}

impl Default for ScSecond {
    fn default() -> Self {
        Self {
            hist: new_histogram(),
            n: 0,
            err: BTreeMap::new(),
            dropped: 0,
        }
    }
}

/// Сборщик метрик агента: шаги — по операциям, итерации — по сценариям.
/// Планировщик держит его в `SharedRecorder`.
///
/// `seconds` (знаменатель `rps_avg` в [`summary`]) сборщик сам не считает: число срезов `tick`
/// не равно длительности нагрузки — после таймлайна идут секунды доработки и хвостовой срез с
/// остатком. Длительность задаёт планировщик через [`Recorder::set_seconds`]; без неё — `0`.
#[derive(Debug, Default)]
pub struct Recorder {
    ops: BTreeMap<String, (OpSecond, OpTotals)>,
    scenarios: BTreeMap<String, (ScSecond, ScTotals)>,
    lag: BTreeMap<String, i64>,
    seconds: u64,
}

pub type SharedRecorder = Arc<Mutex<Recorder>>;

fn bump(map: &mut BTreeMap<String, u64>, code: StepCode) {
    *map.entry(code.as_str().to_string()).or_insert(0) += 1;
}

fn add_counts(into: &mut BTreeMap<String, u64>, from: &BTreeMap<String, u64>) {
    for (k, v) in from {
        *into.entry(k.clone()).or_insert(0) += v;
    }
}

fn pct(h: &Histogram<u64>, q: f64) -> f64 {
    if h.is_empty() {
        0.0
    } else {
        h.value_at_quantile(q) as f64 / 1000.0
    }
}

fn max_ms(h: &Histogram<u64>) -> f64 {
    if h.is_empty() {
        0.0
    } else {
        h.max() as f64 / 1000.0
    }
}

fn record_us(h: &mut Histogram<u64>, latency_us: u64) {
    h.saturating_record(latency_us.max(HIST_LOW_US));
}

impl Recorder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Шаг: операция получает задержку, байты и код ошибки.
    pub fn record(&mut self, s: Sample) {
        let (sec, total) = self.ops.entry(s.operation).or_default();
        record_us(&mut sec.hist, s.latency_us);
        record_us(&mut total.hist, s.latency_us);
        sec.n += 1;
        total.n += 1;
        sec.bytes_in += s.bytes_in;
        sec.bytes_out += s.bytes_out;
        total.bytes_in += s.bytes_in;
        total.bytes_out += s.bytes_out;
        if let Some(code) = s.error {
            bump(&mut sec.err, code);
            bump(&mut total.err, code);
        }
    }

    /// Итерация сценария целиком: суммарная задержка шагов и первая ошибка.
    pub fn iteration(&mut self, scenario: &str, latency_us: u64, error: Option<StepCode>) {
        let (sec, total) = self.scenarios.entry(scenario.to_string()).or_default();
        record_us(&mut sec.hist, latency_us);
        record_us(&mut total.hist, latency_us);
        sec.n += 1;
        total.n += 1;
        if let Some(code) = error {
            bump(&mut sec.err, code);
            bump(&mut total.err, code);
        }
    }

    /// Итерация не стартовала из-за `max_in_flight`: считается сценарию и всем его операциям.
    pub fn dropped(&mut self, scenario: &str, op_ids: &[String]) {
        let (sec, total) = self.scenarios.entry(scenario.to_string()).or_default();
        sec.dropped += 1;
        total.dropped += 1;
        for op in op_ids {
            let (sec, total) = self.ops.entry(op.clone()).or_default();
            sec.dropped += 1;
            total.dropped += 1;
        }
    }

    /// Лаг потребителя операции `kafka_consume`; попадает в ближайший `tick` (операция
    /// появляется в срезах, даже если её шаги ещё не ходили).
    pub fn lag(&mut self, op: &str, lag: i64) {
        self.ops.entry(op.to_string()).or_default();
        self.lag.insert(op.to_string(), lag);
    }

    /// Срез секунды `t`: возвращает метрики и обнуляет посекундные счётчики. На `seconds` итога
    /// не влияет (см. [`Recorder::set_seconds`]).
    pub fn tick(&mut self, t: u64) -> Tick {
        let mut ops = Vec::with_capacity(self.ops.len());
        for (id, (sec, total)) in &mut self.ops {
            let sec = std::mem::take(sec);
            total.per_second_max = total.per_second_max.max(sec.n);
            ops.push(OpTick {
                id: id.clone(),
                n: sec.n,
                err: sec.err,
                dropped: sec.dropped,
                p50: pct(&sec.hist, 0.50),
                p90: pct(&sec.hist, 0.90),
                p95: pct(&sec.hist, 0.95),
                p99: pct(&sec.hist, 0.99),
                max: max_ms(&sec.hist),
                bytes_in: sec.bytes_in,
                bytes_out: sec.bytes_out,
                lag: self.lag.get(id).copied(),
                hist: tick_hist(&sec.hist),
            });
        }
        let mut scenarios = Vec::with_capacity(self.scenarios.len());
        for (id, (sec, total)) in &mut self.scenarios {
            let sec = std::mem::take(sec);
            total.per_second_max = total.per_second_max.max(sec.n);
            scenarios.push(ScenarioTick {
                id: id.clone(),
                n: sec.n,
                err: sec.err,
                dropped: sec.dropped,
                p50: pct(&sec.hist, 0.50),
                p95: pct(&sec.hist, 0.95),
                p99: pct(&sec.hist, 0.99),
                max: max_ms(&sec.hist),
                hist: tick_hist(&sec.hist),
            });
        }
        Tick { t, ops, scenarios }
    }

    /// Длительность нагрузки в секундах для итога (`Totals::seconds`).
    pub fn set_seconds(&mut self, seconds: u64) {
        self.seconds = seconds;
    }

    /// Итоги за весь прогон (посекундные остатки, не закрытые `tick`, не входят).
    pub fn finish(&self) -> Totals {
        Totals {
            ops: self
                .ops
                .iter()
                .map(|(k, (_, t))| (k.clone(), t.clone()))
                .collect(),
            scenarios: self
                .scenarios
                .iter()
                .map(|(k, (_, t))| (k.clone(), t.clone()))
                .collect(),
            seconds: self.seconds,
        }
    }
}

/// Складывает итоги агентов: гистограммы — точно (`Histogram::add`), счётчики — суммой,
/// `per_second_max` — суммой (агенты идут по одному таймлайну, пики совпадают по секундам),
/// `seconds` — максимум. Гистограмма, которую нельзя сложить (границы не те, что у
/// [`new_histogram`], — например, испорченный файл агента), — ошибка с id операции или сценария.
pub fn merge_totals(parts: &[Totals]) -> Result<Totals, String> {
    let mut out = Totals::default();
    for p in parts {
        out.seconds = out.seconds.max(p.seconds);
        for (id, t) in &p.ops {
            let o = out.ops.entry(id.clone()).or_default();
            o.hist
                .add(&t.hist)
                .map_err(|e| format!("operation `{id}`: cannot merge histograms: {e:?}"))?;
            o.n += t.n;
            add_counts(&mut o.err, &t.err);
            o.dropped += t.dropped;
            o.bytes_in += t.bytes_in;
            o.bytes_out += t.bytes_out;
            o.per_second_max += t.per_second_max;
        }
        for (id, t) in &p.scenarios {
            let o = out.scenarios.entry(id.clone()).or_default();
            o.hist
                .add(&t.hist)
                .map_err(|e| format!("scenario `{id}`: cannot merge histograms: {e:?}"))?;
            o.n += t.n;
            add_counts(&mut o.err, &t.err);
            o.dropped += t.dropped;
            o.per_second_max += t.per_second_max;
        }
    }
    Ok(out)
}

/// Взвешенное по `n` среднее перцентиля: приближение для живого графика одной секунды.
fn weighted(acc: f64, acc_n: u64, value: f64, n: u64) -> f64 {
    if acc_n + n == 0 {
        0.0
    } else {
        (acc * acc_n as f64 + value * n as f64) / (acc_n + n) as f64
    }
}

/// Слитая гистограмма частей одной операции или сценария. Части без замеров (`n == 0`)
/// пропускаются; `None`, если у какой-то части с замерами `hist` нет, он не декодируется
/// или не складывается.
fn merged_hist(parts: &[(u64, Option<&str>)]) -> Option<Histogram<u64>> {
    let mut out = new_histogram();
    for (n, hist) in parts {
        if *n == 0 {
            continue;
        }
        let h = decode_hist((*hist)?).ok()?;
        out.add(&h).ok()?;
    }
    Some(out)
}

/// Одна секунда разных агентов: `n`, `err`, `dropped`, байты — суммой, `lag` — суммой.
///
/// Перцентили и `max` — по одному из двух путей для каждой операции и сценария:
/// - точный: у каждой части с замерами (`n > 0`) есть декодируемый `hist` — гистограммы
///   складываются, `p50`…`p99` и `max` берутся из слитой, а `hist` результата — она же
///   (`None`, если замеров не было вовсе);
/// - приближённый (агент без `hist`, испорченная или несовместимая гистограмма):
///   перцентили — взвешенное по `n` среднее, `max` — максимум, `hist` результата — `None`.
pub fn merge_ticks(parts: &[Tick]) -> Tick {
    let t = parts.first().map(|p| p.t).unwrap_or(0);
    let mut ops: BTreeMap<String, OpTick> = BTreeMap::new();
    let mut scenarios: BTreeMap<String, ScenarioTick> = BTreeMap::new();
    // Гистограммы частей: id → (n, hist) по порядку частей.
    let mut op_hists: BTreeMap<String, Vec<(u64, Option<&str>)>> = BTreeMap::new();
    let mut sc_hists: BTreeMap<String, Vec<(u64, Option<&str>)>> = BTreeMap::new();
    for p in parts {
        for o in &p.ops {
            op_hists
                .entry(o.id.clone())
                .or_default()
                .push((o.n, o.hist.as_deref()));
            match ops.get_mut(&o.id) {
                None => {
                    ops.insert(o.id.clone(), o.clone());
                }
                Some(acc) => {
                    acc.p50 = weighted(acc.p50, acc.n, o.p50, o.n);
                    acc.p90 = weighted(acc.p90, acc.n, o.p90, o.n);
                    acc.p95 = weighted(acc.p95, acc.n, o.p95, o.n);
                    acc.p99 = weighted(acc.p99, acc.n, o.p99, o.n);
                    acc.max = acc.max.max(o.max);
                    acc.n += o.n;
                    add_counts(&mut acc.err, &o.err);
                    acc.dropped += o.dropped;
                    acc.bytes_in += o.bytes_in;
                    acc.bytes_out += o.bytes_out;
                    acc.lag = match (acc.lag, o.lag) {
                        (None, None) => None,
                        (a, b) => Some(a.unwrap_or(0) + b.unwrap_or(0)),
                    };
                }
            }
        }
        for s in &p.scenarios {
            sc_hists
                .entry(s.id.clone())
                .or_default()
                .push((s.n, s.hist.as_deref()));
            match scenarios.get_mut(&s.id) {
                None => {
                    scenarios.insert(s.id.clone(), s.clone());
                }
                Some(acc) => {
                    acc.p50 = weighted(acc.p50, acc.n, s.p50, s.n);
                    acc.p95 = weighted(acc.p95, acc.n, s.p95, s.n);
                    acc.p99 = weighted(acc.p99, acc.n, s.p99, s.n);
                    acc.max = acc.max.max(s.max);
                    acc.n += s.n;
                    add_counts(&mut acc.err, &s.err);
                    acc.dropped += s.dropped;
                }
            }
        }
    }
    for (id, acc) in &mut ops {
        acc.hist = None;
        if let Some(h) = merged_hist(&op_hists[id]) {
            acc.p50 = pct(&h, 0.50);
            acc.p90 = pct(&h, 0.90);
            acc.p95 = pct(&h, 0.95);
            acc.p99 = pct(&h, 0.99);
            acc.max = max_ms(&h);
            acc.hist = tick_hist(&h);
        }
    }
    for (id, acc) in &mut scenarios {
        acc.hist = None;
        if let Some(h) = merged_hist(&sc_hists[id]) {
            acc.p50 = pct(&h, 0.50);
            acc.p95 = pct(&h, 0.95);
            acc.p99 = pct(&h, 0.99);
            acc.max = max_ms(&h);
            acc.hist = tick_hist(&h);
        }
    }
    Tick {
        t,
        ops: ops.into_values().collect(),
        scenarios: scenarios.into_values().collect(),
    }
}

/// Сводка: `rps_avg = n / seconds`, `rps_peak = per_second_max`, `err_pct` — доля ошибок,
/// `achieved_pct = rps_avg / target × 100` для операций с целью.
pub fn summary(
    totals: &Totals,
    targets: &BTreeMap<String, f64>,
    status: RunStatus,
) -> AgentSummary {
    let seconds = totals.seconds;
    let rate = |n: u64| {
        if seconds == 0 {
            0.0
        } else {
            n as f64 / seconds as f64
        }
    };
    let ops = totals
        .ops
        .iter()
        .map(|(id, t)| {
            let errors: u64 = t.err.values().sum();
            let rps_avg = rate(t.n);
            let target = targets.get(id).copied();
            OpSummary {
                id: id.clone(),
                n: t.n,
                rps_avg,
                rps_peak: t.per_second_max as f64,
                err: t.err.clone(),
                err_pct: if t.n == 0 {
                    0.0
                } else {
                    errors as f64 / t.n as f64 * 100.0
                },
                dropped: t.dropped,
                p50: pct(&t.hist, 0.50),
                p90: pct(&t.hist, 0.90),
                p95: pct(&t.hist, 0.95),
                p99: pct(&t.hist, 0.99),
                max: max_ms(&t.hist),
                target,
                achieved_pct: target.filter(|t| *t > 0.0).map(|t| rps_avg / t * 100.0),
            }
        })
        .collect();
    let scenarios = totals
        .scenarios
        .iter()
        .map(|(id, t)| ScenarioSummary {
            id: id.clone(),
            n: t.n,
            err: t.err.clone(),
            p50: pct(&t.hist, 0.50),
            p95: pct(&t.hist, 0.95),
            p99: pct(&t.hist, 0.99),
            max: max_ms(&t.hist),
        })
        .collect();
    AgentSummary {
        status,
        ops,
        scenarios,
        seconds,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OP: &str = "http:orders:POST /v1/orders";
    const SC: &str = "order-flow";

    fn sample(latency_us: u64, error: Option<StepCode>) -> Sample {
        Sample {
            operation: OP.into(),
            scenario: SC.into(),
            latency_us,
            bytes_in: 10,
            bytes_out: 20,
            error,
        }
    }

    /// Детерминированный, но неровный ряд задержек: от 100 мкс до ~2 с.
    fn latencies(n: u64) -> Vec<u64> {
        (0..n).map(|i| 100 + (i * 7919) % 2_000_000).collect()
    }

    #[test]
    fn merged_histograms_match_the_histogram_of_all_samples() {
        let all = latencies(5000);
        let mut whole = Recorder::new();
        let mut a = Recorder::new();
        let mut b = Recorder::new();
        for (i, l) in all.iter().enumerate() {
            whole.record(sample(*l, None));
            whole.iteration(SC, *l, None);
            let part = if i % 3 == 0 { &mut a } else { &mut b };
            part.record(sample(*l, None));
            part.iteration(SC, *l, None);
        }
        for r in [&mut whole, &mut a, &mut b] {
            r.tick(1);
            r.set_seconds(1);
        }
        let merged = merge_totals(&[a.finish(), b.finish()]).unwrap();
        let one = whole.finish();
        assert_eq!(merged.ops[OP].hist, one.ops[OP].hist);
        assert_eq!(merged.scenarios[SC].hist, one.scenarios[SC].hist);
        let targets = BTreeMap::new();
        let m = summary(&merged, &targets, RunStatus::Completed);
        let o = summary(&one, &targets, RunStatus::Completed);
        assert_eq!(
            (m.ops[0].p50, m.ops[0].p95, m.ops[0].p99, m.ops[0].max),
            (o.ops[0].p50, o.ops[0].p95, o.ops[0].p99, o.ops[0].max)
        );
        assert_eq!(m.ops[0].n, 5000);
        assert_eq!(m.ops[0].rps_peak, 5000.0, "пики агентов складываются");
        assert_eq!(m.seconds, 1);
        assert!(m.ops[0].p99 > m.ops[0].p50 && m.ops[0].max >= m.ops[0].p99);
        assert!(
            m.ops[0].max <= 2001.0 && m.ops[0].max > 1990.0,
            "{}",
            m.ops[0].max
        );
    }

    #[test]
    fn tick_returns_the_second_and_resets_it() {
        let mut r = Recorder::new();
        r.record(sample(1_000, None));
        r.record(sample(3_000, Some(StepCode::Http5xx)));
        r.record(sample(2_000, None));
        r.iteration(SC, 6_000, Some(StepCode::Http5xx));
        r.lag("kafka_consume:orders.created", 42);
        r.dropped(SC, &[OP.to_string()]);
        let t1 = r.tick(1);
        assert_eq!(t1.t, 1);
        let op = &t1.ops[0];
        assert_eq!((op.id.as_str(), op.n, op.dropped), (OP, 3, 1));
        assert_eq!(op.err, BTreeMap::from([("HTTP_5xx".to_string(), 1)]));
        assert_eq!((op.bytes_in, op.bytes_out), (30, 60));
        assert!((op.p50 - 2.0).abs() < 0.01, "{}", op.p50);
        assert!((op.max - 3.0).abs() < 0.01, "{}", op.max);
        assert_eq!(op.lag, None, "лаг только у своей операции");
        let sc = &t1.scenarios[0];
        assert_eq!((sc.id.as_str(), sc.n, sc.dropped), (SC, 1, 1));
        assert!((sc.max - 6.0).abs() < 0.01);
        let t2 = r.tick(2);
        assert_eq!(t2.ops[0].n, 0);
        assert_eq!(t2.ops[0].max, 0.0);
        assert!(t2.ops[0].err.is_empty());
        assert_eq!(t2.scenarios[0].n, 0);
        assert_eq!(r.finish().seconds, 0, "срезы секунд не считает");
        r.set_seconds(2);
        let totals = r.finish();
        assert_eq!(totals.seconds, 2);
        assert_eq!(totals.ops[OP].n, 3);
        assert_eq!(totals.ops[OP].per_second_max, 3);
        assert_eq!(totals.ops[OP].dropped, 1);
        assert_eq!(totals.scenarios[SC].err["HTTP_5xx"], 1);
    }

    #[test]
    fn lag_is_reported_for_its_operation() {
        let mut r = Recorder::new();
        r.record(Sample {
            operation: "kafka_consume:orders.created".into(),
            ..sample(500, None)
        });
        r.lag("kafka_consume:orders.created", 7);
        let t = r.tick(1);
        assert_eq!(t.ops[0].lag, Some(7));
        let t = r.tick(2);
        assert_eq!(
            t.ops[0].lag,
            Some(7),
            "последнее известное значение держится"
        );
    }

    #[test]
    fn summary_computes_rates_and_shares() {
        let mut r = Recorder::new();
        for i in 0..1000u64 {
            r.record(sample(1_000, (i % 10 == 0).then_some(StepCode::Timeout)));
            if i % 100 == 99 {
                r.tick(i / 100 + 1);
            }
        }
        r.set_seconds(10);
        let totals = r.finish();
        assert_eq!(totals.seconds, 10);
        let targets = BTreeMap::from([(OP.to_string(), 200.0)]);
        let s = summary(&totals, &targets, RunStatus::Stopped);
        assert_eq!(s.status, RunStatus::Stopped);
        let op = &s.ops[0];
        assert_eq!(op.rps_avg, 100.0);
        assert_eq!(op.rps_peak, 100.0);
        assert_eq!(op.err["TIMEOUT"], 100);
        assert_eq!(op.err_pct, 10.0);
        assert_eq!(op.target, Some(200.0));
        assert_eq!(op.achieved_pct, Some(50.0));
        let s = summary(&totals, &BTreeMap::new(), RunStatus::Completed);
        assert_eq!(s.ops[0].target, None);
        assert_eq!(s.ops[0].achieved_pct, None);
        let empty = summary(&Totals::default(), &targets, RunStatus::Failed);
        assert!(empty.ops.is_empty() && empty.seconds == 0);
    }

    #[test]
    fn merge_ticks_sums_counts_and_keeps_max() {
        let mk = |n: u64, p50: f64, max: f64, lag: Option<i64>| Tick {
            t: 5,
            ops: vec![OpTick {
                id: OP.into(),
                n,
                err: BTreeMap::from([("HTTP_5xx".to_string(), 1)]),
                dropped: 1,
                p50,
                p90: p50,
                p95: p50,
                p99: p50,
                max,
                bytes_in: 10,
                bytes_out: 5,
                lag,
                hist: None,
            }],
            scenarios: vec![ScenarioTick {
                id: SC.into(),
                n,
                err: BTreeMap::new(),
                dropped: 0,
                p50,
                p95: p50,
                p99: p50,
                max,
                hist: None,
            }],
        };
        let m = merge_ticks(&[mk(100, 10.0, 50.0, Some(3)), mk(300, 20.0, 40.0, None)]);
        assert_eq!(m.t, 5);
        let op = &m.ops[0];
        assert_eq!(
            (op.n, op.dropped, op.bytes_in, op.bytes_out),
            (400, 2, 20, 10)
        );
        assert_eq!(op.err["HTTP_5xx"], 2);
        assert_eq!(op.max, 50.0);
        assert!((op.p50 - 17.5).abs() < 1e-9, "взвешенное по n: {}", op.p50);
        assert_eq!(op.lag, Some(3));
        assert_eq!(
            op.hist, None,
            "без гистограмм — приближение и пустой `hist`"
        );
        assert_eq!(m.scenarios[0].n, 400);
        assert_eq!(m.scenarios[0].hist, None);
        assert_eq!(merge_ticks(&[]).t, 0);
    }

    /// Два агента с непересекающимися задержками в одной секунде и один агент со всеми.
    fn split_seconds() -> (Tick, Tick, Tick) {
        let mut whole = Recorder::new();
        let mut a = Recorder::new();
        let mut b = Recorder::new();
        // Агент `a` — быстрые ответы (1–2 мс), агент `b` — медленные (100–300 мс).
        for i in 0..1000u64 {
            let l = 1_000 + i;
            whole.record(sample(l, None));
            whole.iteration(SC, l, None);
            a.record(sample(l, None));
            a.iteration(SC, l, None);
        }
        for i in 0..100u64 {
            let l = 100_000 + i * 2_000;
            whole.record(sample(l, None));
            whole.iteration(SC, l, None);
            b.record(sample(l, None));
            b.iteration(SC, l, None);
        }
        (whole.tick(3), a.tick(3), b.tick(3))
    }

    #[test]
    fn tick_carries_the_histogram_of_the_second() {
        let (whole, a, _) = split_seconds();
        let h = decode_hist(whole.ops[0].hist.as_deref().unwrap()).unwrap();
        assert_eq!(h.len(), 1100);
        let h = decode_hist(a.scenarios[0].hist.as_deref().unwrap()).unwrap();
        assert_eq!(h.len(), 1000);
        // Секунда без замеров гистограмму не несёт.
        let mut r = Recorder::new();
        r.dropped(SC, &[OP.to_string()]);
        let t = r.tick(1);
        assert_eq!(
            (t.ops[0].hist.as_deref(), t.scenarios[0].hist.as_deref()),
            (None, None)
        );
        assert!(decode_hist("not base64!").is_err());
        assert!(decode_hist("AAAA").is_err());
    }

    #[test]
    fn merge_ticks_takes_percentiles_from_merged_histograms() {
        let (whole, a, b) = split_seconds();
        let m = merge_ticks(&[a.clone(), b.clone()]);
        let (op, one) = (&m.ops[0], &whole.ops[0]);
        assert_eq!(
            (op.p50, op.p90, op.p95, op.p99, op.max),
            (one.p50, one.p90, one.p95, one.p99, one.max)
        );
        assert_eq!(op.n, 1100);
        let (sc, one) = (&m.scenarios[0], &whole.scenarios[0]);
        assert_eq!(
            (sc.p50, sc.p95, sc.p99, sc.max),
            (one.p50, one.p95, one.p99, one.max)
        );
        // Слитая гистограмма уходит дальше и содержит все замеры секунды.
        let h = decode_hist(op.hist.as_deref().unwrap()).unwrap();
        assert_eq!(h.len(), op.n);
        let h = decode_hist(sc.hist.as_deref().unwrap()).unwrap();
        assert_eq!(h.len(), sc.n);
        // Взвешенное среднее здесь заметно врёт: p99 медленного агента размылся бы до ~30 мс.
        assert!(op.p99 > 250.0, "{}", op.p99);
        // Часть без замеров не мешает точному пути.
        let mut idle = a.clone();
        for o in &mut idle.ops {
            (o.n, o.hist) = (0, None);
        }
        for s in &mut idle.scenarios {
            (s.n, s.hist) = (0, None);
        }
        let m2 = merge_ticks(&[a, b, idle]);
        assert_eq!(m2.ops[0].p99, m.ops[0].p99);
        assert!(m2.ops[0].hist.is_some());
    }

    #[test]
    fn merge_ticks_falls_back_to_weighted_without_a_histogram() {
        let (_, a, b) = split_seconds();
        let weighted_p50 =
            |x: f64, nx: u64, y: f64, ny: u64| (x * nx as f64 + y * ny as f64) / (nx + ny) as f64;
        let mut old = b.clone();
        old.ops[0].hist = None;
        old.scenarios[0].hist = Some("garbage".into());
        let m = merge_ticks(&[a.clone(), old]);
        let op = &m.ops[0];
        assert_eq!(op.hist, None);
        assert_eq!(op.n, 1100);
        assert!(
            (op.p50 - weighted_p50(a.ops[0].p50, 1000, b.ops[0].p50, 100)).abs() < 1e-9,
            "{}",
            op.p50
        );
        assert_eq!(op.max, a.ops[0].max.max(b.ops[0].max));
        let sc = &m.scenarios[0];
        assert_eq!(sc.hist, None, "неразборчивый `hist` — тоже приближение");
        assert!(
            (sc.p99 - weighted_p50(a.scenarios[0].p99, 1000, b.scenarios[0].p99, 100)).abs() < 1e-9
        );
    }

    #[test]
    fn merge_totals_reports_histograms_it_cannot_add() {
        let mut wide = Histogram::<u64>::new_with_bounds(1, 3_600_000_000, 3).unwrap();
        wide.record(1_000_000_000).unwrap();
        let bad = Totals {
            ops: BTreeMap::from([(
                OP.to_string(),
                OpTotals {
                    hist: wide.clone(),
                    n: 1,
                    ..Default::default()
                },
            )]),
            scenarios: BTreeMap::new(),
            seconds: 1,
        };
        let e = merge_totals(&[Totals::default(), bad]).unwrap_err();
        assert!(e.contains(OP) && e.starts_with("operation"), "{e}");
        let bad = Totals {
            ops: BTreeMap::new(),
            scenarios: BTreeMap::from([(
                SC.to_string(),
                ScTotals {
                    hist: wide,
                    n: 1,
                    ..Default::default()
                },
            )]),
            seconds: 1,
        };
        let e = merge_totals(&[bad]).unwrap_err();
        assert!(e.contains(SC) && e.starts_with("scenario"), "{e}");
    }
}
