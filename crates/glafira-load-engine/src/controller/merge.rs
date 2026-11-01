//! Сведение тиков разных агентов в общий посекундный ряд (спека §4.5 п. 5).
//! Саму секунду сводит `metrics::merge_ticks` (точно по гистограммам тиков, иначе приближённо);
//! здесь — когда секунду закрывать и сумма счётчиков за прогон.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use tokio::time::Instant;

use crate::agent_api::Tick;
use crate::metrics::{Totals, merge_ticks};

fn sum_err(into: &mut BTreeMap<String, u64>, from: &BTreeMap<String, u64>) {
    for (code, n) in from {
        *into.entry(code.clone()).or_insert(0) += n;
    }
}

struct Pending {
    parts: BTreeMap<String, Tick>,
    first_seen: Instant,
}

/// Буфер секунд: секунда закрывается, когда получены тики всех живых агентов или прошло `hold`
/// с её первого тика; закрытие секунды закрывает и все более ранние. Закрытые тики отдаются по
/// порядку `t`, каждая секунда — один раз. Тик уже закрытой секунды (агент дослал его после
/// переподключения) в ряд не идёт, а копится в «опоздавших» (`take_late`), чтобы счётчики итога
/// его не потеряли.
pub struct TickMerger {
    live: BTreeSet<String>,
    pending: BTreeMap<u64, Pending>,
    hold: Duration,
    /// Все секунды младше `next` уже закрыты.
    next: u64,
    late: Vec<Tick>,
}

impl TickMerger {
    pub fn new(agents: impl IntoIterator<Item = String>, hold: Duration) -> Self {
        Self {
            live: agents.into_iter().collect(),
            pending: BTreeMap::new(),
            hold,
            next: 0,
            late: Vec::new(),
        }
    }

    /// Закрывает все секунды до `upto` включительно.
    fn close_upto(&mut self, upto: u64) -> Vec<Tick> {
        let rest = match upto.checked_add(1) {
            Some(after) => self.pending.split_off(&after),
            None => BTreeMap::new(),
        };
        let closed = std::mem::replace(&mut self.pending, rest);
        if let Some(last) = closed.keys().next_back() {
            self.next = self.next.max(last.saturating_add(1));
        }
        closed
            .into_values()
            .map(|p| merge_ticks(&p.parts.into_values().collect::<Vec<_>>()))
            .collect()
    }

    /// Старшая секунда, для которой есть тики всех живых агентов.
    fn complete_upto(&self) -> Option<u64> {
        self.pending
            .iter()
            .rev()
            .find(|(_, p)| self.live.iter().all(|a| p.parts.contains_key(a)))
            .map(|(t, _)| *t)
    }

    fn close_complete(&mut self) -> Vec<Tick> {
        match self.complete_upto() {
            Some(upto) => self.close_upto(upto),
            None => Vec::new(),
        }
    }

    /// Тик агента; возвращает закрытые секунды. Тики агентов вне списка живых не учитываются.
    pub fn push(&mut self, agent: &str, tick: Tick, now: Instant) -> Vec<Tick> {
        if !self.live.contains(agent) {
            return Vec::new();
        }
        if tick.t < self.next {
            self.late.push(tick);
            return Vec::new();
        }
        self.pending
            .entry(tick.t)
            .or_insert_with(|| Pending {
                parts: BTreeMap::new(),
                first_seen: now,
            })
            .parts
            .insert(agent.to_string(), tick);
        self.close_complete()
    }

    /// Закрывает секунды, которые ждут дольше `hold`.
    pub fn expire(&mut self, now: Instant) -> Vec<Tick> {
        let upto = self
            .pending
            .iter()
            .rev()
            .find(|(_, p)| now.duration_since(p.first_seen) >= self.hold)
            .map(|(t, _)| *t);
        match upto {
            Some(upto) => self.close_upto(upto),
            None => Vec::new(),
        }
    }

    /// Агент потерян или завершился: его тики больше не ждут; секунды, ставшие полными, закрываются.
    pub fn drop_agent(&mut self, agent: &str) -> Vec<Tick> {
        self.live.remove(agent);
        if self.live.is_empty() {
            return self.flush();
        }
        self.close_complete()
    }

    /// Закрывает всё, что осталось.
    pub fn flush(&mut self) -> Vec<Tick> {
        self.close_upto(u64::MAX)
    }

    /// Тики уже закрытых секунд, пришедшие после закрытия; отдаются один раз.
    pub fn take_late(&mut self) -> Vec<Tick> {
        std::mem::take(&mut self.late)
    }
}

#[derive(Debug, Default, Clone)]
struct OpCounters {
    n: u64,
    err: BTreeMap<String, u64>,
    dropped: u64,
    bytes_in: u64,
    bytes_out: u64,
    per_second_max: u64,
}

#[derive(Debug, Default, Clone)]
struct ScCounters {
    n: u64,
    err: BTreeMap<String, u64>,
    dropped: u64,
    per_second_max: u64,
}

/// Сумма сведённых тиков за прогон: счётчики итога, которых нет в `hist.bin`
/// (`err`, `dropped`, байты), и `n`/`per_second_max` по всем агентам вместе.
#[derive(Debug, Default, Clone)]
pub struct Accumulator {
    ops: BTreeMap<String, OpCounters>,
    scenarios: BTreeMap<String, ScCounters>,
    seconds: u64,
}

impl Accumulator {
    /// Добавляет тик. `per_second_max` — наибольшее `n` одного тика: для сведённых тиков это пик
    /// всех агентов за секунду; опоздавший тик (`TickMerger::take_late`) пик не поднимает выше,
    /// чем его собственное `n`.
    pub fn add(&mut self, tick: &Tick) {
        self.seconds = self.seconds.max(tick.t.saturating_add(1));
        for o in &tick.ops {
            let c = self.ops.entry(o.id.clone()).or_default();
            c.n += o.n;
            sum_err(&mut c.err, &o.err);
            c.dropped += o.dropped;
            c.bytes_in += o.bytes_in;
            c.bytes_out += o.bytes_out;
            c.per_second_max = c.per_second_max.max(o.n);
        }
        for s in &tick.scenarios {
            let c = self.scenarios.entry(s.id.clone()).or_default();
            c.n += s.n;
            sum_err(&mut c.err, &s.err);
            c.dropped += s.dropped;
            c.per_second_max = c.per_second_max.max(s.n);
        }
    }

    /// Переносит счётчики в `totals` (гистограммы из `hist.bin` остаются); операции и сценарии
    /// без гистограммы получают пустую. `seconds` — наибольшее из итога и тиков.
    pub fn fill_counters(&self, totals: &mut Totals) {
        totals.seconds = totals.seconds.max(self.seconds);
        for (id, c) in &self.ops {
            let e = totals.ops.entry(id.clone()).or_default();
            e.n = c.n;
            e.err = c.err.clone();
            e.dropped = c.dropped;
            e.bytes_in = c.bytes_in;
            e.bytes_out = c.bytes_out;
            e.per_second_max = c.per_second_max;
        }
        for (id, c) in &self.scenarios {
            let e = totals.scenarios.entry(id.clone()).or_default();
            e.n = c.n;
            e.err = c.err.clone();
            e.dropped = c.dropped;
            e.per_second_max = c.per_second_max;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_api::{OpTick, ScenarioTick};
    use crate::metrics::{Totals, encode_hist, new_histogram};

    fn op(id: &str, n: u64, p50: f64, max: f64, lag: Option<i64>) -> OpTick {
        let mut err = BTreeMap::new();
        if n > 0 {
            err.insert("HTTP_5xx".to_string(), 1);
        }
        OpTick {
            id: id.into(),
            n,
            err,
            dropped: 1,
            p50,
            p90: p50,
            p95: p50,
            p99: p50,
            max,
            bytes_in: 10,
            bytes_out: 20,
            lag,
            hist: None,
        }
    }

    fn tick(t: u64, n: u64, p50: f64, max: f64) -> Tick {
        Tick {
            t,
            ops: vec![op("x", n, p50, max, Some(5))],
            scenarios: vec![ScenarioTick {
                id: "s".into(),
                n,
                err: BTreeMap::new(),
                dropped: 0,
                p50,
                p95: p50,
                p99: p50,
                max,
                hist: None,
            }],
        }
    }

    /// Тик с гистограммой из `latencies_us` — путь точного слияния `metrics::merge_ticks`.
    fn tick_with_hist(t: u64, latencies_us: &[u64]) -> Tick {
        let mut h = new_histogram();
        latencies_us.iter().for_each(|v| h.record(*v).unwrap());
        let mut tk = tick(t, latencies_us.len() as u64, 0.0, 0.0);
        tk.ops[0].hist = Some(encode_hist(&h));
        tk
    }

    #[tokio::test(start_paused = true)]
    async fn merger_closes_a_second_when_all_live_agents_reported_or_after_hold() {
        let now = tokio::time::Instant::now;
        let mut m = TickMerger::new(["a".to_string(), "b".to_string()], Duration::from_secs(2));
        assert!(m.push("a", tick(0, 1, 1.0, 1.0), now()).is_empty());
        let closed = m.push("b", tick(0, 2, 1.0, 1.0), now());
        assert_eq!(closed.len(), 1);
        assert_eq!((closed[0].t, closed[0].ops[0].n), (0, 3));
        assert!(m.push("a", tick(1, 1, 1.0, 1.0), now()).is_empty());
        tokio::time::advance(Duration::from_secs(1)).await;
        assert!(m.expire(now()).is_empty());
        tokio::time::advance(Duration::from_secs(1)).await;
        let closed = m.expire(now());
        assert_eq!((closed.len(), closed[0].t, closed[0].ops[0].n), (1, 1, 1));
        assert!(m.push("a", tick(2, 1, 1.0, 1.0), now()).is_empty());
        let closed = m.push("b", tick(3, 1, 1.0, 1.0), now());
        assert!(closed.is_empty(), "секунда 3 ждёт a");
        let closed = m.push("a", tick(3, 1, 1.0, 1.0), now());
        assert_eq!(
            closed.iter().map(|t| t.t).collect::<Vec<_>>(),
            vec![2, 3],
            "ранняя секунда закрывается первой"
        );
        assert!(m.push("a", tick(4, 1, 1.0, 1.0), now()).is_empty());
        let closed = m.drop_agent("b");
        assert_eq!(
            (closed.len(), closed[0].t),
            (1, 4),
            "после потери b тик a закрывает секунду"
        );
        assert!(
            m.push("b", tick(5, 9, 1.0, 1.0), now()).is_empty(),
            "потерянный агент не ждётся"
        );
        let closed = m.push("a", tick(5, 1, 1.0, 1.0), now());
        assert_eq!((closed.len(), closed[0].ops[0].n), (1, 1));
        assert_eq!(
            m.push("a", tick(6, 1, 1.0, 1.0), now()).len(),
            1,
            "единственный живой агент закрывает секунду сразу"
        );
        assert!(m.drop_agent("a").is_empty() && m.flush().is_empty());
        let mut two = TickMerger::new(["y".to_string(), "z".to_string()], Duration::from_secs(2));
        two.push("z", tick(9, 1, 1.0, 1.0), now());
        assert!(two.drop_agent("z").is_empty(), "секунда 9 ждёт y");
        assert_eq!(
            two.drop_agent("y").len(),
            1,
            "drop последнего агента закрывает всё"
        );
        let mut slow = TickMerger::new(["y".to_string(), "z".to_string()], Duration::from_secs(2));
        slow.push("z", tick(9, 1, 1.0, 1.0), now());
        assert_eq!(slow.flush().len(), 1, "flush закрывает неполные секунды");
    }

    #[tokio::test(start_paused = true)]
    async fn merger_keeps_the_series_monotonic_and_hands_out_late_ticks() {
        let now = tokio::time::Instant::now;
        let mut m = TickMerger::new(["a".to_string(), "b".to_string()], Duration::from_secs(2));
        m.push("a", tick(0, 1, 1.0, 1.0), now());
        m.push("a", tick(1, 1, 1.0, 1.0), now());
        tokio::time::advance(Duration::from_secs(2)).await;
        let closed = m.expire(now());
        assert_eq!(closed.iter().map(|t| t.t).collect::<Vec<_>>(), vec![0, 1]);
        // b переподключился и дослал старые секунды: в ряд они не идут, но не теряются.
        assert!(m.push("b", tick(0, 4, 1.0, 1.0), now()).is_empty());
        assert!(m.push("b", tick(1, 5, 1.0, 1.0), now()).is_empty());
        let late = m.take_late();
        assert_eq!(
            late.iter().map(|t| (t.t, t.ops[0].n)).collect::<Vec<_>>(),
            vec![(0, 4), (1, 5)]
        );
        assert!(m.take_late().is_empty());
        m.push("b", tick(2, 1, 1.0, 1.0), now());
        let closed = m.push("a", tick(2, 1, 1.0, 1.0), now());
        assert_eq!((closed.len(), closed[0].t, closed[0].ops[0].n), (1, 2, 2));
    }

    #[test]
    fn merger_uses_exact_histogram_merge_of_metrics() {
        let mut m = TickMerger::new(["a".to_string(), "b".to_string()], Duration::from_secs(2));
        let at = tokio::time::Instant::now();
        m.push("a", tick_with_hist(0, &[1_000; 10]), at);
        let closed = m.push("b", tick_with_hist(0, &[100_000; 30]), at);
        let x = &closed[0].ops[0];
        assert_eq!(x.n, 40);
        assert!(x.hist.is_some(), "слитая гистограмма идёт дальше");
        // Точный p50 из слитой гистограммы — 100 мс, а не взвешенное среднее 75,25 мс.
        assert!((x.p50 - 100.0).abs() < 1.0, "{}", x.p50);
        assert!((x.p90 - 100.0).abs() < 1.0, "{}", x.p90);
    }

    #[test]
    fn accumulator_fills_counters_into_totals() {
        let mut acc = Accumulator::default();
        acc.add(&tick(0, 10, 1.0, 1.0));
        acc.add(&tick(1, 30, 1.0, 1.0));
        let mut totals = Totals {
            ops: BTreeMap::new(),
            scenarios: BTreeMap::new(),
            seconds: 0,
        };
        acc.fill_counters(&mut totals);
        let x = &totals.ops["x"];
        assert_eq!(
            (
                x.n,
                x.dropped,
                x.bytes_in,
                x.bytes_out,
                x.per_second_max,
                x.err["HTTP_5xx"]
            ),
            (40, 2, 20, 40, 30, 2)
        );
        assert!(
            x.hist.is_empty(),
            "операция без hist.bin получает пустую гистограмму"
        );
        let s = &totals.scenarios["s"];
        assert_eq!((s.n, s.per_second_max, totals.seconds), (40, 30, 2));
    }

    #[test]
    fn accumulator_overrides_hist_counts_and_keeps_histograms() {
        let mut h = new_histogram();
        h.record(5_000).unwrap();
        let mut totals = Totals::default();
        totals.ops.insert(
            "x".into(),
            crate::metrics::OpTotals {
                hist: h.clone(),
                n: 1,
                per_second_max: 1,
                ..Default::default()
            },
        );
        totals.seconds = 7;
        let mut acc = Accumulator::default();
        acc.add(&tick(2, 3, 1.0, 1.0));
        acc.fill_counters(&mut totals);
        let x = &totals.ops["x"];
        assert_eq!((x.n, x.per_second_max, x.hist.len()), (3, 3, 1));
        assert_eq!(
            totals.seconds, 7,
            "seconds — наибольшее из hist.bin и тиков"
        );
    }
}
