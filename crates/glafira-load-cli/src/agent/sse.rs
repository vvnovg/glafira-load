//! Буфер событий прогона за 120 с с монотонным `id`, живой поток через broadcast и SSE
//! с `Last-Event-ID` (спека §4.3, отступление 9).

use std::collections::VecDeque;
use std::convert::Infallible;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use futures::{Stream, StreamExt};
use glafira_load_engine::agent_api::{Event, RunStatus};
use tokio::sync::broadcast;
use tokio::sync::broadcast::error::RecvError;

use super::not_found;
use super::runs::AgentState;

pub const RETENTION: Duration = Duration::from_secs(120);
const CHANNEL: usize = 4096;

struct Stored {
    id: u64,
    at: Instant,
    event: Event,
}

struct Inner {
    buf: VecDeque<Stored>,
    next_id: u64,
    done: Option<RunStatus>,
}

/// События одного прогона: буфер для переподключений и broadcast для живых подписчиков.
pub struct EventLog {
    inner: Mutex<Inner>,
    tx: broadcast::Sender<(u64, Event)>,
    retention: Duration,
}

/// Снимок буфера: события с `id > after`, `gap` — часть пропущена (старее буфера), `done` — прогон завершён.
pub struct Replay {
    pub events: Vec<(u64, Event)>,
    pub gap: bool,
    pub done: bool,
}

impl Default for EventLog {
    fn default() -> Self {
        Self::new()
    }
}

impl EventLog {
    pub fn new() -> Self {
        Self::with_retention(RETENTION)
    }

    pub fn with_retention(retention: Duration) -> Self {
        let (tx, _) = broadcast::channel(CHANNEL);
        Self {
            inner: Mutex::new(Inner {
                buf: VecDeque::new(),
                next_id: 1,
                done: None,
            }),
            tx,
            retention,
        }
    }

    pub fn push(&self, event: Event) -> u64 {
        self.push_at(event, Instant::now())
    }

    /// Кладёт событие с меткой `at` (для тестов вытеснения); после `done` события не принимаются.
    pub fn push_at(&self, event: Event, at: Instant) -> u64 {
        let mut g = self.inner.lock().unwrap();
        if g.done.is_some() {
            return g.next_id - 1;
        }
        let id = g.next_id;
        g.next_id += 1;
        if let Event::Done(summary) = &event {
            g.done = Some(summary.status);
        }
        let retention = self.retention;
        while g
            .buf
            .front()
            .is_some_and(|s| at.saturating_duration_since(s.at) > retention)
        {
            g.buf.pop_front();
        }
        g.buf.push_back(Stored {
            id,
            at,
            event: event.clone(),
        });
        let _ = self.tx.send((id, event));
        id
    }

    pub fn status(&self) -> Option<RunStatus> {
        self.inner.lock().unwrap().done
    }

    pub fn is_done(&self) -> bool {
        self.status().is_some()
    }

    pub fn last_id(&self) -> u64 {
        self.inner.lock().unwrap().next_id - 1
    }

    pub fn replay(&self, after: Option<u64>) -> Replay {
        let g = self.inner.lock().unwrap();
        let after = after.unwrap_or(0);
        let gap = match g.buf.front() {
            Some(first) => after + 1 < first.id,
            None => after < g.next_id - 1,
        };
        let events = g
            .buf
            .iter()
            .filter(|s| s.id > after)
            .map(|s| (s.id, s.event.clone()))
            .collect();
        Replay {
            events,
            gap,
            done: g.done.is_some(),
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<(u64, Event)> {
        self.tx.subscribe()
    }
}

/// Имя SSE-события — тег `type` из `agent_api::Event`.
pub fn event_name(event: &Event) -> &'static str {
    match event {
        Event::Tick(_) => "tick",
        Event::Phase(_) => "phase",
        Event::Error(_) => "error",
        Event::Done(_) => "done",
        Event::Gap => "gap",
    }
}

fn to_sse(id: Option<u64>, event: &Event) -> SseEvent {
    let mut sse = SseEvent::default().event(event_name(event));
    if let Some(id) = id {
        sse = sse.id(id.to_string());
    }
    match sse.json_data(event) {
        Ok(sse) => sse,
        Err(_) => SseEvent::default().event("gap").data("{\"type\":\"gap\"}"),
    }
}

struct Live {
    pending: VecDeque<(Option<u64>, Event)>,
    rx: broadcast::Receiver<(u64, Event)>,
    /// Последний отданный id: отсекает дубли между снимком и подпиской.
    floor: u64,
    finished: bool,
}

/// Поток SSE: сначала снимок буфера (с `gap`, если `last_id` старее буфера), затем живые события; `done` закрывает поток.
pub fn stream(
    log: Arc<EventLog>,
    last_id: Option<u64>,
) -> impl Stream<Item = Result<SseEvent, Infallible>> + Send + 'static {
    log_stream(log, last_id).map(|(id, event)| Ok(to_sse(id, &event)))
}

/// События потока SSE с их `id` (`gap` — без `id`). `last_id` больше последнего известного
/// (клиент слушал другой прогон или агент до рестарта) — как подключение без `Last-Event-ID`.
pub fn log_stream(
    log: Arc<EventLog>,
    last_id: Option<u64>,
) -> impl Stream<Item = (Option<u64>, Event)> + Send + 'static {
    // подписка до снимка: событие между ними придёт по каналу и отсечётся по `floor`
    let rx = log.subscribe();
    // `last_id()` растёт монотонно: id не больше него после подписки не устареет
    let last_id = last_id.filter(|&id| id <= log.last_id());
    let replay = log.replay(last_id);
    let mut pending = VecDeque::new();
    if replay.gap {
        pending.push_back((None, Event::Gap));
    }
    let mut floor = last_id.unwrap_or(0);
    for (id, event) in replay.events {
        floor = floor.max(id);
        pending.push_back((Some(id), event));
    }
    let live = Live {
        pending,
        rx,
        floor,
        finished: replay.done,
    };
    futures::stream::unfold((live, log), |(mut st, log)| async move {
        loop {
            if let Some((id, event)) = st.pending.pop_front() {
                if matches!(event, Event::Done(_)) {
                    st.finished = true;
                    st.pending.clear();
                }
                return Some(((id, event), (st, log)));
            }
            if st.finished {
                return None;
            }
            match st.rx.recv().await {
                Ok((id, event)) if id > st.floor => {
                    st.floor = id;
                    st.pending.push_back((Some(id), event));
                }
                Ok(_) => {}
                Err(RecvError::Lagged(_)) => {
                    // отстали от канала — добираем из буфера
                    let r = log.replay(Some(st.floor));
                    if r.gap {
                        st.pending.push_back((None, Event::Gap));
                    }
                    for (id, event) in r.events {
                        st.floor = st.floor.max(id);
                        st.pending.push_back((Some(id), event));
                    }
                    if r.done && st.pending.is_empty() {
                        st.finished = true;
                    }
                }
                Err(RecvError::Closed) => st.finished = true,
            }
        }
    })
}

/// `GET /v1/runs/{id}/events`.
pub async fn events(
    State(state): State<Arc<AgentState>>,
    Path(run_id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let Some(log) = state.find_log(&run_id) else {
        return not_found(&run_id);
    };
    let last_id = headers
        .get("last-event-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse::<u64>().ok());
    Sse::new(stream(log, last_id))
        .keep_alive(KeepAlive::default())
        .into_response()
}

#[cfg(test)]
mod tests {
    use glafira_load_engine::agent_api::{AgentSummary, Tick};

    use super::*;

    fn tick(t: u64) -> Event {
        Event::Tick(Tick {
            t,
            ops: vec![],
            scenarios: vec![],
        })
    }

    fn done() -> Event {
        Event::Done(AgentSummary {
            status: RunStatus::Completed,
            ops: vec![],
            scenarios: vec![],
            seconds: 2,
        })
    }

    #[tokio::test]
    async fn replay_and_stream_end_with_done() {
        let log = Arc::new(EventLog::new());
        assert_eq!(
            (log.push(tick(1)), log.push(tick(2)), log.push(done())),
            (1, 2, 3)
        );
        assert_eq!(log.push(tick(3)), 3, "после done ничего не принимается");
        assert!(matches!(log.status(), Some(RunStatus::Completed)));
        let r = log.replay(Some(1));
        assert_eq!(
            r.events.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
            [2, 3]
        );
        assert!(!r.gap && r.done);
        assert_eq!(stream(log.clone(), None).collect::<Vec<_>>().await.len(), 3);
        assert_eq!(
            stream(log.clone(), Some(2)).collect::<Vec<_>>().await.len(),
            1
        );
        assert_eq!(stream(log, Some(3)).collect::<Vec<_>>().await.len(), 0);
    }

    #[tokio::test]
    async fn evicted_events_produce_gap() {
        let log = Arc::new(EventLog::new());
        let now = Instant::now();
        log.push_at(tick(1), now - Duration::from_secs(200));
        log.push_at(tick(2), now - Duration::from_secs(200));
        log.push_at(tick(3), now);
        let r = log.replay(Some(1));
        assert!(r.gap);
        assert_eq!(r.events.iter().map(|(id, _)| *id).collect::<Vec<_>>(), [3]);
        assert!(!log.replay(Some(2)).gap);
        assert!(
            log.replay(None).gap,
            "свежее подключение тоже пропустило начало"
        );
    }

    /// Имена событий потока `log_stream` до его конца.
    async fn names(log: Arc<EventLog>, last_id: Option<u64>) -> Vec<&'static str> {
        log_stream(log, last_id)
            .map(|(_, ev)| event_name(&ev))
            .collect()
            .await
    }

    #[tokio::test]
    async fn stream_from_an_evicted_id_starts_with_gap() {
        let log = Arc::new(EventLog::new());
        let now = Instant::now();
        log.push_at(tick(1), now - Duration::from_secs(200));
        log.push_at(tick(2), now - Duration::from_secs(200));
        log.push_at(tick(3), now);
        log.push_at(done(), now);
        assert_eq!(names(log.clone(), Some(1)).await, ["gap", "tick", "done"]);
        let ids: Vec<Option<u64>> = log_stream(log, Some(1)).map(|(id, _)| id).collect().await;
        assert_eq!(ids, [None, Some(3), Some(4)]);
    }

    #[tokio::test]
    async fn last_event_id_beyond_the_last_is_a_fresh_connection() {
        let log = Arc::new(EventLog::new());
        log.push(tick(1));
        log.push(tick(2));
        log.push(done());
        assert_eq!(names(log.clone(), Some(99)).await, ["tick", "tick", "done"]);
        assert_eq!(
            stream(log.clone(), Some(99))
                .collect::<Vec<_>>()
                .await
                .len(),
            3
        );
        // живой прогон: поток с чужим id не молчит, а отдаёт новые события
        let live = Arc::new(EventLog::new());
        live.push(tick(1));
        let collected = tokio::spawn(names(live.clone(), Some(42)));
        tokio::time::sleep(Duration::from_millis(50)).await;
        live.push(tick(2));
        live.push(done());
        assert_eq!(collected.await.unwrap(), ["tick", "tick", "done"]);
    }

    #[tokio::test]
    async fn live_subscriber_receives_events_pushed_later() {
        let log = Arc::new(EventLog::new());
        log.push(tick(1));
        let collected = tokio::spawn(stream(log.clone(), None).collect::<Vec<_>>());
        tokio::time::sleep(Duration::from_millis(50)).await;
        log.push(tick(2));
        log.push(done());
        assert_eq!(collected.await.unwrap().len(), 3);
    }
}
