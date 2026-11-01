//! Планировщик открытого цикла по RPS (спека L2 §5.1): задача на сценарий, интервалы
//! Poisson или Constant, потолок `max_in_flight`, посекундные `tick`, смена фаз, остановка с
//! доработкой активных итераций; режим `smoke` — каждый сценарий один раз последовательно.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use glafira_load_model::profile::Point;
use glafira_load_model::suite::{Arrival, OnFail, Suite};
use rand::Rng;
use tokio::sync::Semaphore;
use tokio::sync::mpsc::Sender;
use tokio::time::{Instant, sleep_until};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::agent_api::{ErrorSample, Event, Mode, PhaseEvent, RunStatus};
use crate::context::{IterationCtx, RunContext};
use crate::errors::StepError;
use crate::executor::StepExecutor;
use crate::metrics::{Sample, SharedRecorder};

#[derive(Debug, Clone)]
pub struct SchedulerConfig {
    /// Цели операций (RPS) уже с долей агента.
    pub targets: BTreeMap<String, f64>,
    pub timeline: Vec<Point>,
    pub total_minutes: f64,
    pub arrival: Arrival,
    pub max_in_flight: u32,
    pub mode: Mode,
    /// Сколько ждать активные итерации после конца или остановки.
    pub drain: Duration,
}

/// Процент цели в секунду `second`: линейная интерполяция таймлайна по минутам;
/// после последней точки — 0.
pub fn pct_at(timeline: &[Point], second: u64) -> f64 {
    let Some(last) = timeline.last() else {
        return 0.0;
    };
    let m = second as f64 / 60.0;
    if m >= last.minute {
        return 0.0;
    }
    for w in timeline.windows(2) {
        let (a, b) = (w[0], w[1]);
        if m >= a.minute && m < b.minute {
            return a.pct + (b.pct - a.pct) * (m - a.minute) / (b.minute - a.minute);
        }
    }
    timeline.first().map(|p| p.pct).unwrap_or(0.0)
}

/// Индекс отрезка таймлайна, в который попадает секунда: последняя точка с
/// `minute × 60 ≤ second`, не дальше предпоследней.
pub fn phase_index(timeline: &[Point], second: u64) -> usize {
    if timeline.len() < 2 {
        return 0;
    }
    let s = second as f64;
    let mut idx = 0;
    for (i, p) in timeline.iter().enumerate().take(timeline.len() - 1) {
        if p.minute * 60.0 <= s {
            idx = i;
        }
    }
    idx
}

/// Не чаще одного примера в секунду на пару (код, операция).
#[derive(Debug, Default)]
pub struct ErrorLimiter {
    last: Mutex<HashMap<(String, String), u64>>,
}

impl ErrorLimiter {
    pub fn allow(&self, code: &str, operation: &str, t: u64) -> bool {
        let mut last = self.last.lock().expect("limiter lock");
        let key = (code.to_string(), operation.to_string());
        match last.get(&key) {
            Some(prev) if *prev == t => false,
            _ => {
                last.insert(key, t);
                true
            }
        }
    }
}

/// Общее для всех задач прогона.
struct Shared {
    suite: Arc<Suite>,
    cfg: SchedulerConfig,
    run_ctx: Arc<RunContext>,
    executor: Arc<dyn StepExecutor>,
    recorder: SharedRecorder,
    events: Sender<Event>,
    limiter: ErrorLimiter,
    /// В `smoke` примеры не ограничиваются.
    limit_errors: bool,
    start: Instant,
}

impl Shared {
    fn elapsed_secs(&self) -> u64 {
        self.start.elapsed().as_secs()
    }

    async fn report(&self, scenario: &str, step: &str, operation: &str, e: StepError) {
        let t = self.elapsed_secs();
        if self.limit_errors && !self.limiter.allow(e.code.as_str(), operation, t) {
            return;
        }
        let _ = self
            .events
            .send(Event::Error(ErrorSample {
                t,
                code: e.code.as_str().to_string(),
                operation: operation.to_string(),
                scenario: scenario.to_string(),
                step: step.to_string(),
                agent: self.run_ctx.agent_id.clone(),
                message: e.message,
                request: e.request,
                response: e.response,
            }))
            .await;
    }
}

/// Одна итерация сценария: шаги по порядку, `on_fail: stop` прерывает после первой ошибки.
async fn run_iteration(sh: &Shared, scenario_index: usize, iteration: u64) {
    let scenario = &sh.suite.scenarios[scenario_index];
    let ids = scenario.step_ids();
    let mut ctx = IterationCtx::new(&sh.run_ctx, iteration);
    let mut total_us = 0u64;
    let mut first_error = None;
    for (step, id) in scenario.steps.iter().zip(&ids) {
        ctx.begin_step(id);
        let operation = step.action.operation().to_string();
        let started = Instant::now();
        let result = sh.executor.execute(step, &mut ctx).await;
        let measured_us = started.elapsed().as_micros() as u64;
        match result {
            Ok(out) => {
                total_us += out.latency_us;
                sh.recorder.lock().expect("recorder lock").record(Sample {
                    operation,
                    scenario: scenario.id.clone(),
                    latency_us: out.latency_us,
                    bytes_in: out.bytes_in,
                    bytes_out: out.bytes_out,
                    error: None,
                });
            }
            Err(e) => {
                total_us += measured_us;
                sh.recorder.lock().expect("recorder lock").record(Sample {
                    operation: operation.clone(),
                    scenario: scenario.id.clone(),
                    latency_us: measured_us,
                    bytes_in: 0,
                    bytes_out: 0,
                    error: Some(e.code),
                });
                if first_error.is_none() {
                    first_error = Some(e.code);
                }
                sh.report(&scenario.id, id, &operation, e).await;
                if scenario.on_fail == OnFail::Stop {
                    break;
                }
            }
        }
    }
    sh.recorder
        .lock()
        .expect("recorder lock")
        .iteration(&scenario.id, total_us, first_error);
}

/// Цикл выпуска итераций одного сценария по расписанию.
async fn scenario_loop(
    sh: Arc<Shared>,
    scenario_index: usize,
    total: Duration,
    semaphore: Arc<Semaphore>,
    tracker: TaskTracker,
    cancel: CancellationToken,
) {
    let scenario = &sh.suite.scenarios[scenario_index];
    let drives = scenario.drives().to_string();
    let drives_steps = scenario
        .steps
        .iter()
        .filter(|s| s.action.operation() == drives)
        .count()
        .max(1) as f64;
    let target = sh.cfg.targets.get(&drives).copied().unwrap_or(0.0);
    let op_ids: Vec<String> = scenario
        .steps
        .iter()
        .map(|s| s.action.operation().to_string())
        .collect();
    let mut iteration = 0u64;
    let mut next = Instant::now();
    loop {
        if cancel.is_cancelled() {
            break;
        }
        let elapsed = sh.start.elapsed();
        if elapsed >= total {
            break;
        }
        let second = elapsed.as_secs();
        let rate = target * pct_at(&sh.cfg.timeline, second) / 100.0 / drives_steps;
        if rate <= 0.0 {
            // Нулевая частота: ждём следующую секунду таймлайна.
            let boundary = sh.start + Duration::from_secs(second + 1);
            tokio::select! {
                _ = sleep_until(boundary) => {}
                _ = cancel.cancelled() => break,
            }
            next = Instant::now();
            continue;
        }
        match semaphore.clone().try_acquire_owned() {
            Ok(permit) => {
                let sh = sh.clone();
                let n = iteration;
                tracker.spawn(async move {
                    run_iteration(&sh, scenario_index, n).await;
                    drop(permit);
                });
            }
            Err(_) => sh
                .recorder
                .lock()
                .expect("recorder lock")
                .dropped(&scenario.id, &op_ids),
        }
        iteration += 1;
        let interval = match sh.cfg.arrival {
            Arrival::Constant => 1.0 / rate,
            Arrival::Poisson => {
                let u: f64 = rand::rng().random();
                -(1.0 - u).ln() / rate
            }
        };
        // Сроки считаются от идеального расписания: округление таймера не копится.
        next += Duration::from_secs_f64(interval);
        tokio::select! {
            _ = sleep_until(next) => {}
            _ = cancel.cancelled() => break,
        }
    }
}

/// Таймер секунд: `tick` и `phase`; возвращает номер последней отданной секунды.
async fn ticker(sh: Arc<Shared>, total: Duration, cancel: CancellationToken) -> u64 {
    let mut t = 0u64;
    let mut phase = phase_index(&sh.cfg.timeline, 0);
    let _ = sh
        .events
        .send(Event::Phase(PhaseEvent {
            t: 0,
            index: phase,
            pct: pct_at(&sh.cfg.timeline, 0),
        }))
        .await;
    loop {
        let next = sh.start + Duration::from_secs(t + 1);
        tokio::select! {
            _ = sleep_until(next) => {}
            _ = cancel.cancelled() => break,
        }
        t += 1;
        let lag = sh.executor.lag().await;
        let tick = {
            let mut rec = sh.recorder.lock().expect("recorder lock");
            for (op, value) in lag {
                rec.lag(&op, value);
            }
            rec.tick(t)
        };
        let _ = sh.events.send(Event::Tick(tick)).await;
        let idx = phase_index(&sh.cfg.timeline, t);
        if idx != phase {
            phase = idx;
            let _ = sh
                .events
                .send(Event::Phase(PhaseEvent {
                    t,
                    index: idx,
                    pct: pct_at(&sh.cfg.timeline, t),
                }))
                .await;
        }
        if Duration::from_secs(t) >= total {
            break;
        }
    }
    t
}

pub struct Scheduler;

impl Scheduler {
    /// Прогон до конца `total_minutes` или `cancel`; события — в `events` (получатель
    /// обязан их читать). Возвращает `Completed` или `Stopped`.
    pub async fn run(
        suite: Arc<Suite>,
        cfg: SchedulerConfig,
        run_ctx: Arc<RunContext>,
        executor: Arc<dyn StepExecutor>,
        recorder: SharedRecorder,
        events: Sender<Event>,
        cancel: CancellationToken,
    ) -> RunStatus {
        let mode = cfg.mode;
        let total = Duration::from_secs_f64(cfg.total_minutes.max(0.0) * 60.0);
        let max_in_flight = cfg.max_in_flight.max(1) as usize;
        let drain = cfg.drain;
        let sh = Arc::new(Shared {
            suite,
            cfg,
            run_ctx,
            executor,
            recorder,
            events,
            limiter: ErrorLimiter::default(),
            limit_errors: mode == Mode::Run,
            start: Instant::now(),
        });
        if mode == Mode::Smoke {
            for i in 0..sh.suite.scenarios.len() {
                if cancel.is_cancelled() {
                    break;
                }
                run_iteration(&sh, i, 0).await;
            }
            let tick = sh.recorder.lock().expect("recorder lock").tick(1);
            let _ = sh.events.send(Event::Tick(tick)).await;
            return if cancel.is_cancelled() {
                RunStatus::Stopped
            } else {
                RunStatus::Completed
            };
        }

        let semaphore = Arc::new(Semaphore::new(max_in_flight));
        let tracker = TaskTracker::new();
        let loops: Vec<_> = (0..sh.suite.scenarios.len())
            .map(|i| {
                tokio::spawn(scenario_loop(
                    sh.clone(),
                    i,
                    total,
                    semaphore.clone(),
                    tracker.clone(),
                    cancel.clone(),
                ))
            })
            .collect();
        let ticker = tokio::spawn(ticker(sh.clone(), total, cancel.clone()));
        for l in loops {
            let _ = l.await;
        }
        // Новые итерации больше не стартуют; активные дорабатывают до `drain`.
        tracker.close();
        let _ = tokio::time::timeout(drain, tracker.wait()).await;
        let last_t = ticker.await.unwrap_or(0);
        // Хвост: итерации, завершившиеся после последнего `tick`.
        let tick = sh.recorder.lock().expect("recorder lock").tick(last_t + 1);
        let _ = sh.events.send(Event::Tick(tick)).await;
        if cancel.is_cancelled() {
            RunStatus::Stopped
        } else {
            RunStatus::Completed
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_api::Tick;
    use crate::errors::StepCode;
    use crate::executor::StepOutcome;
    use crate::metrics::Recorder;
    use async_trait::async_trait;
    use glafira_load_model::suite::{Action, Defaults, Expect, HttpStep, Scenario, Step};
    use std::sync::atomic::{AtomicU64, Ordering};
    use tokio::sync::mpsc;

    const OP_A: &str = "http:s:GET /a";
    const OP_B: &str = "http:s:GET /b";

    /// Фиктивный исполнитель: считает вызовы по операции, умеет ждать и падать.
    #[derive(Default)]
    struct Fake {
        calls: Mutex<BTreeMap<String, u64>>,
        total: AtomicU64,
        delay: Option<Duration>,
        fail: Option<String>,
    }

    impl Fake {
        fn count(&self, op: &str) -> u64 {
            self.calls.lock().unwrap().get(op).copied().unwrap_or(0)
        }
    }

    #[async_trait]
    impl StepExecutor for Fake {
        async fn execute(
            &self,
            step: &Step,
            _ctx: &mut IterationCtx<'_>,
        ) -> Result<StepOutcome, StepError> {
            let op = step.action.operation().to_string();
            *self.calls.lock().unwrap().entry(op.clone()).or_insert(0) += 1;
            self.total.fetch_add(1, Ordering::SeqCst);
            if let Some(d) = self.delay {
                tokio::time::sleep(d).await;
            }
            if self.fail.as_deref() == Some(op.as_str()) {
                return Err(StepError::new(StepCode::Http5xx, "status 500"));
            }
            Ok(StepOutcome {
                latency_us: 1000,
                bytes_in: 10,
                bytes_out: 5,
                status: Some(200),
                body: None,
            })
        }

        async fn lag(&self) -> BTreeMap<String, i64> {
            BTreeMap::from([("kafka_consume:t".to_string(), 3)])
        }
    }

    fn http(op: &str) -> Step {
        Step {
            id: None,
            action: Action::Http(HttpStep {
                operation: op.into(),
                path: BTreeMap::new(),
                query: BTreeMap::new(),
                headers: BTreeMap::new(),
                body: None,
                body_text: None,
                timeout_ms: None,
            }),
            expect: Expect::default(),
            extract: BTreeMap::new(),
        }
    }

    fn suite(scenarios: Vec<Scenario>) -> Arc<Suite> {
        Arc::new(Suite {
            format: 1,
            name: "t".into(),
            secrets: vec![],
            data: BTreeMap::new(),
            defaults: Defaults::default(),
            scenarios,
        })
    }

    fn scenario(id: &str, ops: &[&str], on_fail: OnFail) -> Scenario {
        Scenario {
            id: id.into(),
            drives: None,
            on_fail,
            steps: ops.iter().map(|o| http(o)).collect(),
        }
    }

    fn flat(seconds: f64, pct: f64) -> Vec<Point> {
        vec![
            Point { minute: 0.0, pct },
            Point {
                minute: seconds / 60.0,
                pct,
            },
        ]
    }

    fn config(
        target: f64,
        timeline: Vec<Point>,
        arrival: Arrival,
        max_in_flight: u32,
    ) -> SchedulerConfig {
        SchedulerConfig {
            targets: BTreeMap::from([(OP_A.to_string(), target)]),
            total_minutes: timeline.last().map(|p| p.minute).unwrap_or(0.0),
            timeline,
            arrival,
            max_in_flight,
            mode: Mode::Run,
            drain: Duration::from_secs(5),
        }
    }

    struct Outcome {
        status: RunStatus,
        events: Vec<Event>,
        recorder: SharedRecorder,
    }

    async fn run(
        suite: Arc<Suite>,
        cfg: SchedulerConfig,
        fake: Arc<Fake>,
        cancel: CancellationToken,
    ) -> Outcome {
        let (tx, mut rx) = mpsc::channel(64);
        let drain = tokio::spawn(async move {
            let mut out = Vec::new();
            while let Some(e) = rx.recv().await {
                out.push(e);
            }
            out
        });
        let recorder: SharedRecorder = Arc::new(Mutex::new(Recorder::new()));
        let status = Scheduler::run(
            suite,
            cfg,
            Arc::new(RunContext::default()),
            fake,
            recorder.clone(),
            tx,
            cancel,
        )
        .await;
        let events = drain.await.unwrap();
        Outcome {
            status,
            events,
            recorder,
        }
    }

    fn ticks(events: &[Event]) -> Vec<&Tick> {
        events
            .iter()
            .filter_map(|e| match e {
                Event::Tick(t) => Some(t),
                _ => None,
            })
            .collect()
    }

    fn op_n(t: &Tick, op: &str) -> u64 {
        t.ops.iter().find(|o| o.id == op).map(|o| o.n).unwrap_or(0)
    }

    #[test]
    fn pct_at_interpolates_and_is_zero_after_the_end() {
        let tl = vec![
            Point {
                minute: 0.0,
                pct: 0.0,
            },
            Point {
                minute: 10.0,
                pct: 100.0,
            },
            Point {
                minute: 30.0,
                pct: 100.0,
            },
            Point {
                minute: 30.0,
                pct: 50.0,
            },
            Point {
                minute: 35.0,
                pct: 50.0,
            },
        ];
        assert_eq!(pct_at(&tl, 0), 0.0);
        assert_eq!(pct_at(&tl, 300), 50.0);
        assert_eq!(pct_at(&tl, 600), 100.0);
        assert_eq!(pct_at(&tl, 1799), 100.0);
        assert_eq!(
            pct_at(&tl, 1800),
            50.0,
            "ступень: две точки с одной минутой"
        );
        assert_eq!(pct_at(&tl, 2099), 50.0);
        assert_eq!(pct_at(&tl, 2100), 0.0, "после конца — 0");
        assert_eq!(pct_at(&tl, 99_999), 0.0);
        assert_eq!(pct_at(&[], 5), 0.0);
        assert_eq!(
            pct_at(
                &[Point {
                    minute: 0.0,
                    pct: 0.0
                }],
                0
            ),
            0.0
        );
        assert_eq!(phase_index(&tl, 0), 0);
        assert_eq!(phase_index(&tl, 600), 1);
        assert_eq!(phase_index(&tl, 1799), 1);
        assert_eq!(phase_index(&tl, 1800), 3);
        assert_eq!(phase_index(&tl, 5000), 3, "не дальше предпоследней точки");
        assert_eq!(phase_index(&[], 5), 0);
    }

    #[test]
    fn limiter_allows_one_per_second_per_code_and_operation() {
        let l = ErrorLimiter::default();
        assert!(l.allow("HTTP_5xx", OP_A, 1));
        assert!(!l.allow("HTTP_5xx", OP_A, 1));
        assert!(l.allow("HTTP_5xx", OP_B, 1));
        assert!(l.allow("TIMEOUT", OP_A, 1));
        assert!(l.allow("HTTP_5xx", OP_A, 2));
    }

    #[tokio::test(start_paused = true)]
    async fn constant_arrival_hits_the_target() {
        let fake = Arc::new(Fake::default());
        let out = run(
            suite(vec![scenario("s", &[OP_A], OnFail::Stop)]),
            config(100.0, flat(10.0, 100.0), Arrival::Constant, 1000),
            fake.clone(),
            CancellationToken::new(),
        )
        .await;
        assert_eq!(out.status, RunStatus::Completed);
        let n = fake.count(OP_A);
        assert!((990..=1010).contains(&n), "{n}");
        let ticks = ticks(&out.events);
        assert_eq!(ticks.len(), 11, "10 секунд и хвост после drain");
        assert!(
            ticks[..10]
                .iter()
                .all(|t| (95..=105).contains(&op_n(t, OP_A))),
            "{:?}",
            ticks.iter().map(|t| op_n(t, OP_A)).collect::<Vec<_>>()
        );
        let kafka = ticks[0]
            .ops
            .iter()
            .find(|o| o.id == "kafka_consume:t")
            .unwrap();
        assert_eq!(
            (kafka.lag, kafka.n),
            (Some(3), 0),
            "лаг исполнителя снимается раз в секунду"
        );
        let totals = out.recorder.lock().unwrap().finish();
        assert_eq!(totals.ops[OP_A].n, n);
        assert_eq!(totals.ops[OP_A].dropped, 0);
        assert_eq!(totals.scenarios["s"].n, n);
        assert!(
            matches!(out.events.first(), Some(Event::Phase(p)) if p.index == 0 && p.pct == 100.0)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn poisson_arrival_is_close_to_the_target() {
        let fake = Arc::new(Fake::default());
        run(
            suite(vec![scenario("s", &[OP_A], OnFail::Stop)]),
            config(100.0, flat(10.0, 100.0), Arrival::Poisson, 1000),
            fake.clone(),
            CancellationToken::new(),
        )
        .await;
        let n = fake.count(OP_A);
        assert!((850..=1150).contains(&n), "{n}");
    }

    #[tokio::test(start_paused = true)]
    async fn driving_steps_divide_the_rate_and_other_steps_follow() {
        let fake = Arc::new(Fake::default());
        run(
            suite(vec![scenario("s", &[OP_A, OP_B, OP_A], OnFail::Stop)]),
            config(100.0, flat(10.0, 100.0), Arrival::Constant, 1000),
            fake.clone(),
            CancellationToken::new(),
        )
        .await;
        let (a, b) = (fake.count(OP_A), fake.count(OP_B));
        assert!((990..=1010).contains(&a), "{a}");
        assert_eq!(a, b * 2);
    }

    #[tokio::test(start_paused = true)]
    async fn ramp_gives_a_growing_series_and_phase_events() {
        let fake = Arc::new(Fake::default());
        let timeline = vec![
            Point {
                minute: 0.0,
                pct: 0.0,
            },
            Point {
                minute: 10.0 / 60.0,
                pct: 100.0,
            },
            Point {
                minute: 12.0 / 60.0,
                pct: 100.0,
            },
        ];
        let out = run(
            suite(vec![scenario("s", &[OP_A], OnFail::Stop)]),
            config(100.0, timeline, Arrival::Constant, 1000),
            fake.clone(),
            CancellationToken::new(),
        )
        .await;
        let series: Vec<u64> = ticks(&out.events).iter().map(|t| op_n(t, OP_A)).collect();
        assert_eq!(series.len(), 13, "{series:?}");
        assert!(series[0] < 15, "{series:?}");
        assert!(series[1] < series[4] && series[4] < series[8], "{series:?}");
        assert!(series[9] >= 85, "{series:?}");
        let phases: Vec<(u64, usize)> = out
            .events
            .iter()
            .filter_map(|e| match e {
                Event::Phase(p) => Some((p.t, p.index)),
                _ => None,
            })
            .collect();
        assert_eq!(phases, vec![(0, 0), (10, 1)]);
    }

    #[tokio::test(start_paused = true)]
    async fn max_in_flight_drops_iterations() {
        let fake = Arc::new(Fake {
            delay: Some(Duration::from_secs(1)),
            ..Default::default()
        });
        let out = run(
            suite(vec![scenario("s", &[OP_A], OnFail::Stop)]),
            config(10.0, flat(5.0, 100.0), Arrival::Constant, 1),
            fake.clone(),
            CancellationToken::new(),
        )
        .await;
        assert_eq!(out.status, RunStatus::Completed);
        let totals = out.recorder.lock().unwrap().finish();
        let calls = fake.count(OP_A);
        assert!((4..=6).contains(&calls), "{calls}");
        assert!(
            totals.ops[OP_A].dropped > 40,
            "{}",
            totals.ops[OP_A].dropped
        );
        assert_eq!(totals.scenarios["s"].dropped, totals.ops[OP_A].dropped);
        assert_eq!(
            totals.ops[OP_A].n, calls,
            "доработавшие итерации учтены в хвосте"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn cancel_stops_the_run_early() {
        let fake = Arc::new(Fake::default());
        let cancel = CancellationToken::new();
        let stopper = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(2500)).await;
            stopper.cancel();
        });
        let started = Instant::now();
        let out = run(
            suite(vec![scenario("s", &[OP_A], OnFail::Stop)]),
            config(100.0, flat(60.0, 100.0), Arrival::Constant, 1000),
            fake.clone(),
            cancel,
        )
        .await;
        assert_eq!(out.status, RunStatus::Stopped);
        assert!(
            started.elapsed() < Duration::from_secs(4),
            "{:?}",
            started.elapsed()
        );
        let n = fake.count(OP_A);
        assert!((240..=260).contains(&n), "{n}");
        let t: Vec<u64> = ticks(&out.events).iter().map(|t| t.t).collect();
        assert_eq!(t, vec![1, 2, 3]);
    }

    #[tokio::test(start_paused = true)]
    async fn smoke_runs_each_scenario_exactly_once() {
        let fake = Arc::new(Fake::default());
        let mut cfg = config(100.0, flat(60.0, 100.0), Arrival::Poisson, 1000);
        cfg.mode = Mode::Smoke;
        let started = Instant::now();
        let out = run(
            suite(vec![
                scenario("one", &[OP_A, OP_B], OnFail::Stop),
                scenario("two", &[OP_A], OnFail::Stop),
            ]),
            cfg,
            fake.clone(),
            CancellationToken::new(),
        )
        .await;
        assert_eq!(out.status, RunStatus::Completed);
        assert_eq!((fake.count(OP_A), fake.count(OP_B)), (2, 1));
        assert!(started.elapsed() < Duration::from_secs(1), "без таймера");
        assert_eq!(ticks(&out.events).len(), 1);
        let totals = out.recorder.lock().unwrap().finish();
        assert_eq!(
            (totals.scenarios["one"].n, totals.scenarios["two"].n),
            (1, 1)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn errors_are_limited_and_on_fail_is_honoured() {
        let fake = Arc::new(Fake {
            fail: Some(OP_A.into()),
            ..Default::default()
        });
        let out = run(
            suite(vec![scenario("stop", &[OP_A, OP_B], OnFail::Stop)]),
            config(50.0, flat(4.0, 100.0), Arrival::Constant, 1000),
            fake.clone(),
            CancellationToken::new(),
        )
        .await;
        assert!(
            (195..=205).contains(&fake.count(OP_A)),
            "{}",
            fake.count(OP_A)
        );
        assert_eq!(fake.count(OP_B), 0, "on_fail: stop прерывает итерацию");
        let errors: Vec<&ErrorSample> = out
            .events
            .iter()
            .filter_map(|e| match e {
                Event::Error(s) => Some(s),
                _ => None,
            })
            .collect();
        assert!(
            (4..=5).contains(&errors.len()),
            "один пример в секунду: {}",
            errors.len()
        );
        assert_eq!(errors[0].code, "HTTP_5xx");
        assert_eq!(
            (
                errors[0].scenario.as_str(),
                errors[0].step.as_str(),
                errors[0].operation.as_str()
            ),
            ("stop", "step1", OP_A)
        );
        let totals = out.recorder.lock().unwrap().finish();
        assert_eq!(totals.ops[OP_A].err["HTTP_5xx"], fake.count(OP_A));
        assert_eq!(totals.scenarios["stop"].err["HTTP_5xx"], fake.count(OP_A));

        let fake = Arc::new(Fake {
            fail: Some(OP_A.into()),
            ..Default::default()
        });
        run(
            suite(vec![scenario("go", &[OP_A, OP_B], OnFail::Continue)]),
            config(50.0, flat(4.0, 100.0), Arrival::Constant, 1000),
            fake.clone(),
            CancellationToken::new(),
        )
        .await;
        assert_eq!(
            fake.count(OP_A),
            fake.count(OP_B),
            "on_fail: continue идёт дальше"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn smoke_mode_sends_every_error_without_the_limiter() {
        let fake = Arc::new(Fake {
            fail: Some(OP_A.into()),
            ..Default::default()
        });
        let mut cfg = config(1.0, flat(1.0, 100.0), Arrival::Constant, 1);
        cfg.mode = Mode::Smoke;
        let out = run(
            suite(vec![
                scenario("one", &[OP_A], OnFail::Stop),
                scenario("two", &[OP_A], OnFail::Stop),
            ]),
            cfg,
            fake,
            CancellationToken::new(),
        )
        .await;
        let errors = out
            .events
            .iter()
            .filter(|e| matches!(e, Event::Error(_)))
            .count();
        assert_eq!(errors, 2);
    }
}
