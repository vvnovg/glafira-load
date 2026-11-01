//! Шаги `kafka_produce` и `kafka_consume` на rdkafka (спека L2 §5.2, решение плана 6).
//!
//! Продюсер один на агента. Потребитель один на все `kafka_consume`-топики сьюта: группа
//! `glafira-load-<run_id>-<agent_id>`, чтение с конца, фоновая задача складывает сообщения в буфер
//! (не больше `BUFFER_MAX` и не старше `BUFFER_TTL`), шаг ждёт в буфере совпадение по `match`.
//! Лаг снимается раз в секунду по смещениям. Рендер шаблонов — до первого `await`.
//! `expect` и `extract` шага `kafka_consume` применяются к значению сообщения (спека §3.4),
//! тексты ошибок и примеры маскируются значениями секретов прогона.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use glafira_load_model::suite::expr::{
    Context, ExprError, parse_template, render_json, render_string,
};
use glafira_load_model::suite::{Action, ConsumeStep, Encoding, Match, ProduceStep, Step};
use rdkafka::client::ClientContext;
use rdkafka::consumer::{
    BaseConsumer, Consumer, ConsumerContext, RebalanceProtocol, StreamConsumer,
};
use rdkafka::message::{Header, Message, OwnedHeaders};
use rdkafka::producer::{FutureProducer, FutureRecord};
use rdkafka::types::RDKafkaRespErr;
use rdkafka::util::Timeout;
use rdkafka::{ClientConfig, Offset, TopicPartitionList};
use serde_json_path::JsonPath;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use crate::avro::AvroCodec;
use crate::context::IterationCtx;
use crate::errors::{StepCode, StepError, sample};
use crate::executor::{StepExecutor, StepOutcome};
use crate::expect::check_expect;
use crate::extract::extract_all;

/// Буфер потребителя: не больше стольких сообщений…
pub const BUFFER_MAX: usize = 10_000;
/// …и не старше стольки.
pub const BUFFER_TTL: Duration = Duration::from_secs(60);
/// Ожидание назначения партиций после `subscribe` (решение плана 6).
pub const ASSIGNMENT_WAIT: Duration = Duration::from_secs(10);
/// Сколько ждать места в очереди продюсера.
pub const PRODUCE_QUEUE_TIMEOUT: Duration = Duration::from_secs(30);
/// Период снятия лага.
pub const LAG_PERIOD: Duration = Duration::from_secs(1);
/// Запрос high watermark партиции при назначении.
const WATERMARK_TIMEOUT: Duration = Duration::from_secs(5);

/// Топик из id операции `kafka_produce:<topic>` или `kafka_consume:<topic>`.
pub fn topic_of(operation: &str) -> Option<&str> {
    operation
        .strip_prefix("kafka_produce:")
        .or_else(|| operation.strip_prefix("kafka_consume:"))
        .filter(|t| !t.is_empty())
}

pub fn consumer_group(run_id: &str, agent_id: &str) -> String {
    format!("glafira-load-{run_id}-{agent_id}")
}

/// Общая часть конфигурации клиентов: `defaults.kafka` сьюта и адрес стенда (адрес побеждает).
fn base_config(bootstrap: &str, defaults: &BTreeMap<String, String>) -> ClientConfig {
    let mut c = ClientConfig::new();
    for (k, v) in defaults {
        c.set(k, v);
    }
    c.set("bootstrap.servers", bootstrap);
    c
}

pub fn producer_config(
    bootstrap: &str,
    defaults: &BTreeMap<String, String>,
    agent_id: &str,
) -> ClientConfig {
    let mut c = base_config(bootstrap, defaults);
    c.set("client.id", format!("glafira-load-{agent_id}"));
    if !defaults.contains_key("message.timeout.ms") {
        c.set("message.timeout.ms", "30000");
    }
    c
}

pub fn consumer_config(
    bootstrap: &str,
    defaults: &BTreeMap<String, String>,
    run_id: &str,
    agent_id: &str,
) -> ClientConfig {
    let mut c = base_config(bootstrap, defaults);
    c.set("client.id", format!("glafira-load-{agent_id}"))
        .set("group.id", consumer_group(run_id, agent_id))
        .set("auto.offset.reset", "latest")
        .set("enable.auto.commit", "false")
        .set("enable.partition.eof", "false");
    c
}

fn step_err(code: StepCode, message: impl Into<String>, request: Option<String>) -> StepError {
    StepError {
        code,
        message: message.into(),
        request,
        response: None,
    }
}

/// Ошибка рендера во время прогона: отсутствует значение шага, набора или колонки.
fn render_err(e: ExprError) -> StepError {
    step_err(
        StepCode::ExtractMissing,
        format!("render: {}", e.message),
        None,
    )
}

fn render_str(text: &str, ctx: &dyn Context) -> Result<String, StepError> {
    let template = parse_template(text).map_err(render_err)?;
    render_string(&template, ctx).map_err(render_err)
}

/// Формы записи секретов в примерах Kafka: значение как есть и экранированное внутри строки
/// JSON (тело сообщения). Пустые значения и дубликаты отбрасываются.
pub fn secret_forms(secrets: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for s in secrets.iter().filter(|s| !s.is_empty()) {
        let quoted = serde_json::Value::String(s.clone()).to_string();
        let escaped = quoted[1..quoted.len() - 1].to_string();
        for form in [s.clone(), escaped] {
            if !out.contains(&form) {
                out.push(form);
            }
        }
    }
    out
}

/// Маскирует секреты в тексте ошибки и примерах, затем обрезает примеры (`errors::sample`).
pub fn mask_error(e: StepError, secrets: &[String]) -> StepError {
    StepError {
        code: e.code,
        message: sample(&e.message, secrets),
        request: e.request.map(|r| sample(&r, secrets)),
        response: e.response.map(|r| sample(&r, secrets)),
    }
}

/// Проверки `expect` и значения `extract` шага `kafka_consume` по значению сообщения;
/// статуса у сообщения нет, задержка — ожидание совпадения.
pub fn check_message(
    step: &Step,
    out: &StepOutcome,
    ctx: &mut IterationCtx<'_>,
) -> Result<(), StepError> {
    check_expect(
        &step.expect,
        None,
        out.body.as_ref(),
        out.latency_us as f64 / 1000.0,
        None,
    )?;
    for (name, value) in extract_all(&step.extract, out.body.as_ref())? {
        ctx.store(&name, value);
    }
    Ok(())
}

/// Строковое представление узла JSON для сравнения с ожидаемым значением `match.jsonpath`.
pub fn as_text(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Сообщение в буфере потребителя.
#[derive(Debug, Clone)]
pub struct Received {
    pub topic: String,
    pub key: Option<String>,
    /// Тело как JSON; не JSON — `None`.
    pub body: Option<serde_json::Value>,
    /// Длина тела в байтах (`bytes_in` шага).
    pub len: u64,
    pub at: Instant,
}

impl Received {
    fn from_message(m: &impl Message) -> Self {
        let payload = m.payload();
        Self {
            topic: m.topic().to_string(),
            key: m.key().map(|k| String::from_utf8_lossy(k).into_owned()),
            body: payload.and_then(|p| serde_json::from_slice(p).ok()),
            len: payload.map_or(0, |p| p.len() as u64),
            at: Instant::now(),
        }
    }
}

/// Условие `match` шага после рендера.
#[derive(Debug)]
pub struct MatchSpec {
    pub topic: String,
    pub key: Option<String>,
    /// Путь → ожидаемое значение строкой; совпадение — любой узел результата, все пути сразу.
    pub jsonpath: Vec<(JsonPath, String)>,
}

impl MatchSpec {
    pub fn render(topic: &str, m: &Match, ctx: &dyn Context) -> Result<Self, StepError> {
        let key = m.key.as_deref().map(|k| render_str(k, ctx)).transpose()?;
        let mut jsonpath = Vec::with_capacity(m.jsonpath.len());
        for (path, expected) in &m.jsonpath {
            let compiled = JsonPath::parse(path).map_err(|e| {
                step_err(
                    StepCode::KafkaConsume,
                    format!("match.jsonpath `{path}`: {e}"),
                    None,
                )
            })?;
            jsonpath.push((compiled, render_str(expected, ctx)?));
        }
        Ok(Self {
            topic: topic.into(),
            key,
            jsonpath,
        })
    }

    pub fn matches(&self, m: &Received) -> bool {
        if m.topic != self.topic {
            return false;
        }
        if let Some(key) = &self.key
            && m.key.as_deref() != Some(key.as_str())
        {
            return false;
        }
        if self.jsonpath.is_empty() {
            return true;
        }
        let Some(body) = &m.body else {
            return false;
        };
        self.jsonpath.iter().all(|(path, expected)| {
            path.query(body)
                .iter()
                .any(|node| as_text(node) == *expected)
        })
    }

    /// Текст условия для примера ошибки по таймауту.
    pub fn describe(&self) -> String {
        let mut out = format!("topic={}", self.topic);
        if let Some(k) = &self.key {
            out.push_str(&format!(" key={k}"));
        }
        for (path, expected) in &self.jsonpath {
            out.push_str(&format!(" {path}={expected}"));
        }
        out
    }
}

/// Буфер последних сообщений: по размеру и по возрасту.
pub struct MessageBuffer {
    items: VecDeque<Received>,
    max: usize,
    ttl: Duration,
}

impl MessageBuffer {
    pub fn new(max: usize, ttl: Duration) -> Self {
        Self {
            items: VecDeque::new(),
            max,
            ttl,
        }
    }

    pub fn push(&mut self, m: Received) {
        self.prune(m.at);
        if self.items.len() >= self.max {
            self.items.pop_front();
        }
        self.items.push_back(m);
    }

    /// Выбрасывает сообщения старше `ttl` относительно `now`.
    pub fn prune(&mut self, now: Instant) {
        while let Some(front) = self.items.front() {
            if now.saturating_duration_since(front.at) > self.ttl {
                self.items.pop_front();
            } else {
                break;
            }
        }
    }

    /// Первое совпавшее сообщение; оно удаляется из буфера.
    pub fn take(&mut self, spec: &MatchSpec) -> Option<Received> {
        let i = self.items.iter().position(|m| spec.matches(m))?;
        self.items.remove(i)
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

/// Шаг `kafka_produce` после рендера.
#[derive(Debug, Clone, PartialEq)]
pub struct ProduceMessage {
    pub operation: String,
    pub topic: String,
    pub key: Option<String>,
    /// `value` как JSON; при `encoding: avro` кодируется по схеме.
    pub value: Option<serde_json::Value>,
    /// `value_text` как есть; при `encoding: avro` разбирается как JSON.
    pub text: Option<String>,
    pub encoding: Encoding,
    pub headers: Vec<(String, String)>,
}

pub fn prepare_produce(step: &ProduceStep, ctx: &dyn Context) -> Result<ProduceMessage, StepError> {
    let topic = topic_of(&step.operation)
        .ok_or_else(|| {
            step_err(
                StepCode::KafkaProduce,
                format!("operation `{}` has no topic", step.operation),
                None,
            )
        })?
        .to_string();
    let key = step
        .key
        .as_deref()
        .map(|k| render_str(k, ctx))
        .transpose()?;
    let value = step
        .value
        .as_ref()
        .map(|v| render_json(v, ctx).map_err(render_err))
        .transpose()?;
    let text = step
        .value_text
        .as_deref()
        .map(|t| render_str(t, ctx))
        .transpose()?;
    let mut headers = Vec::with_capacity(step.headers.len());
    for (k, v) in &step.headers {
        headers.push((k.clone(), render_str(v, ctx)?));
    }
    Ok(ProduceMessage {
        operation: step.operation.clone(),
        topic,
        key,
        value,
        text,
        encoding: step.encoding,
        headers,
    })
}

/// Шаг `kafka_consume` после рендера.
#[derive(Debug)]
pub struct ConsumeRequest {
    pub topic: String,
    pub spec: MatchSpec,
    pub timeout: Duration,
}

pub fn prepare_consume(step: &ConsumeStep, ctx: &dyn Context) -> Result<ConsumeRequest, StepError> {
    let topic = topic_of(&step.operation)
        .ok_or_else(|| {
            step_err(
                StepCode::KafkaConsume,
                format!("operation `{}` has no topic", step.operation),
                None,
            )
        })?
        .to_string();
    let spec = MatchSpec::render(&topic, &step.match_, ctx)?;
    Ok(ConsumeRequest {
        topic,
        spec,
        timeout: Duration::from_millis(step.timeout_ms),
    })
}

/// Контекст потребителя: при назначении партиций начало чтения фиксируется на high watermark
/// каждой партиции. Одного `auto.offset.reset=latest` мало: «конец» разрешается асинхронно уже
/// после назначения, и сообщения первых итераций, отправленные в этот промежуток, пропускались бы.
/// Если watermark не получен, у партиции остаётся `latest`.
struct StartAtEnd;

impl ClientContext for StartAtEnd {}

impl ConsumerContext for StartAtEnd {
    fn rebalance(
        &self,
        consumer: &BaseConsumer<Self>,
        err: RDKafkaRespErr,
        tpl: &mut TopicPartitionList,
    ) {
        let cooperative = matches!(
            consumer.rebalance_protocol(),
            RebalanceProtocol::Cooperative
        );
        let result = match err {
            RDKafkaRespErr::RD_KAFKA_RESP_ERR__ASSIGN_PARTITIONS => {
                pin_to_end(consumer, tpl);
                if cooperative {
                    consumer.incremental_assign(tpl)
                } else {
                    consumer.assign(tpl)
                }
            }
            RDKafkaRespErr::RD_KAFKA_RESP_ERR__REVOKE_PARTITIONS if cooperative => {
                consumer.incremental_unassign(tpl)
            }
            RDKafkaRespErr::RD_KAFKA_RESP_ERR__REVOKE_PARTITIONS => consumer.unassign(),
            other => {
                tracing::warn!(error = ?other, "kafka rebalance");
                consumer.unassign()
            }
        };
        if let Err(e) = result {
            tracing::warn!(error = %e, "kafka rebalance");
        }
    }
}

/// Смещение каждой назначенной партиции — её текущий high watermark.
fn pin_to_end(consumer: &BaseConsumer<StartAtEnd>, tpl: &mut TopicPartitionList) {
    let partitions: Vec<(String, i32)> = tpl
        .elements()
        .iter()
        .map(|e| (e.topic().to_string(), e.partition()))
        .collect();
    for (topic, partition) in partitions {
        let pinned = consumer
            .fetch_watermarks(&topic, partition, WATERMARK_TIMEOUT)
            .and_then(|(_low, high)| {
                tpl.set_partition_offset(&topic, partition, Offset::Offset(high))
            });
        if let Err(e) = pinned {
            tracing::warn!(error = %e, topic = %topic, partition, "kafka start offset");
        }
    }
}

type HubConsumer = StreamConsumer<StartAtEnd>;

/// Потребитель всех consume-топиков прогона и его фоновые задачи.
struct ConsumerHub {
    consumer: HubConsumer,
    topics: Vec<String>,
    buffer: Mutex<MessageBuffer>,
    /// Счётчик принятых сообщений: будит ожидающие шаги.
    seq: watch::Sender<u64>,
    /// Операция `kafka_consume:<topic>` → лаг, обновляется раз в секунду.
    lag: Mutex<BTreeMap<String, i64>>,
    cancel: CancellationToken,
}

async fn read_loop(hub: Arc<ConsumerHub>) {
    loop {
        tokio::select! {
            _ = hub.cancel.cancelled() => break,
            next = hub.consumer.recv() => match next {
                Ok(m) => {
                    let received = Received::from_message(&m);
                    hub.buffer.lock().expect("consumer buffer").push(received);
                    hub.seq.send_modify(|n| *n += 1);
                }
                Err(e) => {
                    tracing::warn!(error = %e, "kafka consumer");
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            },
        }
    }
}

/// Лаг по каждому consume-топику: high watermark минус позиция потребителя, сумма по партициям.
/// Партиция без позиции (ещё ничего не прочитано) считается догнавшей.
fn compute_lag(consumer: &HubConsumer, topics: &[String]) -> BTreeMap<String, i64> {
    let mut out: BTreeMap<String, i64> = topics
        .iter()
        .map(|t| (format!("kafka_consume:{t}"), 0))
        .collect();
    let Ok(position) = consumer.position() else {
        return out;
    };
    for e in position.elements() {
        let topic = e.topic();
        if !topics.iter().any(|t| t == topic) {
            continue;
        }
        let Ok((_low, high)) =
            consumer.fetch_watermarks(topic, e.partition(), Duration::from_secs(1))
        else {
            continue;
        };
        let pos = e.offset().to_raw().filter(|o| *o >= 0).unwrap_or(high);
        *out.entry(format!("kafka_consume:{topic}")).or_insert(0) += (high - pos).max(0);
    }
    out
}

async fn lag_loop(hub: Arc<ConsumerHub>) {
    let mut tick = tokio::time::interval(LAG_PERIOD);
    loop {
        tokio::select! {
            _ = hub.cancel.cancelled() => break,
            _ = tick.tick() => {}
        }
        let h = hub.clone();
        // fetch_watermarks блокирует до секунды — вне исполнителя tokio.
        let computed = tokio::task::spawn_blocking(move || compute_lag(&h.consumer, &h.topics))
            .await
            .unwrap_or_default();
        *hub.lag.lock().expect("lag map") = computed;
    }
}

async fn wait_assignment(consumer: &HubConsumer, limit: Duration) -> Result<(), String> {
    let started = Instant::now();
    loop {
        match consumer.assignment() {
            Ok(a) if a.count() > 0 => return Ok(()),
            Ok(_) => {}
            Err(e) => return Err(format!("kafka consumer assignment: {e}")),
        }
        if started.elapsed() > limit {
            return Err(format!(
                "kafka consumer got no partitions within {} s",
                limit.as_secs()
            ));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Исполнитель шагов Kafka одного прогона на агенте.
pub struct KafkaExecutor {
    producer: FutureProducer,
    hub: Option<Arc<ConsumerHub>>,
    avro: AvroCodec,
    /// Формы секретов прогона для маскирования ошибок (`secret_forms`).
    secrets: Vec<String>,
    cancel: CancellationToken,
}

impl KafkaExecutor {
    /// `consume_topics` — топики всех `kafka_consume` сьюта (пусто — потребитель не создаётся);
    /// `avro_schemas` — id операции `kafka_produce` с `encoding: avro` → текст `.avsc`.
    /// Возвращает после назначения партиций (до `ASSIGNMENT_WAIT`), чтобы не терять первые сообщения.
    pub async fn new(
        bootstrap: &str,
        defaults: &BTreeMap<String, String>,
        consume_topics: &[String],
        run_id: &str,
        agent_id: &str,
        schema_registry: Option<String>,
        avro_schemas: &BTreeMap<String, String>,
    ) -> Result<Self, String> {
        let producer: FutureProducer = producer_config(bootstrap, defaults, agent_id)
            .create()
            .map_err(|e| format!("kafka producer: {e}"))?;
        let avro = AvroCodec::new(schema_registry, avro_schemas)?;
        let cancel = CancellationToken::new();
        let hub = if consume_topics.is_empty() {
            None
        } else {
            let consumer: HubConsumer = consumer_config(bootstrap, defaults, run_id, agent_id)
                .create_with_context(StartAtEnd)
                .map_err(|e| format!("kafka consumer: {e}"))?;
            let names: Vec<&str> = consume_topics.iter().map(String::as_str).collect();
            consumer
                .subscribe(&names)
                .map_err(|e| format!("kafka subscribe: {e}"))?;
            let (seq, _) = watch::channel(0u64);
            let hub = Arc::new(ConsumerHub {
                consumer,
                topics: consume_topics.to_vec(),
                buffer: Mutex::new(MessageBuffer::new(BUFFER_MAX, BUFFER_TTL)),
                seq,
                lag: Mutex::new(BTreeMap::new()),
                cancel: cancel.clone(),
            });
            // Ребаланс идёт внутри poll: читающая задача стартует до ожидания назначения.
            tokio::spawn(read_loop(hub.clone()));
            if let Err(e) = wait_assignment(&hub.consumer, ASSIGNMENT_WAIT).await {
                cancel.cancel();
                return Err(e);
            }
            tokio::spawn(lag_loop(hub.clone()));
            Some(hub)
        };
        Ok(Self {
            producer,
            hub,
            avro,
            secrets: Vec::new(),
            cancel,
        })
    }

    /// Значения секретов прогона: ими маскируются тексты ошибок и примеры.
    pub fn with_secrets(mut self, secrets: &[String]) -> Self {
        self.secrets = secret_forms(secrets);
        self
    }

    /// Останавливает фоновые задачи потребителя.
    pub fn shutdown(&self) {
        self.cancel.cancel();
    }

    /// Байты сообщения и текст для примера ошибки.
    async fn payload(&self, m: &ProduceMessage) -> Result<(Vec<u8>, String), StepError> {
        match m.encoding {
            Encoding::Json => Ok(match (&m.value, &m.text) {
                (Some(v), _) => {
                    let s = v.to_string();
                    (s.clone().into_bytes(), s)
                }
                (None, Some(t)) => (t.clone().into_bytes(), t.clone()),
                (None, None) => (Vec::new(), String::new()),
            }),
            Encoding::Avro => {
                let json = match (&m.value, &m.text) {
                    (Some(v), _) => v.clone(),
                    (None, Some(t)) => serde_json::from_str(t).map_err(|e| {
                        step_err(
                            StepCode::AvroEncode,
                            format!("value_text is not JSON: {e}"),
                            Some(t.clone()),
                        )
                    })?,
                    (None, None) => serde_json::Value::Null,
                };
                // Пример — полное значение: маскирование до обрезки (`mask_error`).
                let bytes = self
                    .avro
                    .encode_message(&m.operation, &m.topic, &json)
                    .await
                    .map_err(|e| StepError {
                        request: Some(json.to_string()),
                        ..e
                    })?;
                Ok((bytes, json.to_string()))
            }
        }
    }

    pub async fn produce(&self, m: ProduceMessage) -> Result<StepOutcome, StepError> {
        self.send(m).await.map_err(|e| mask_error(e, &self.secrets))
    }

    async fn send(&self, m: ProduceMessage) -> Result<StepOutcome, StepError> {
        let (payload, preview) = self.payload(&m).await?;
        let mut headers = OwnedHeaders::new();
        for (k, v) in &m.headers {
            headers = headers.insert(Header {
                key: k,
                value: Some(v.as_str()),
            });
        }
        let queue = Timeout::After(PRODUCE_QUEUE_TIMEOUT);
        let started = Instant::now();
        let sent = match &m.key {
            Some(k) => {
                let record = FutureRecord::<String, Vec<u8>>::to(&m.topic)
                    .key(k)
                    .payload(&payload)
                    .headers(headers);
                self.producer.send(record, queue).await
            }
            None => {
                let record = FutureRecord::<(), Vec<u8>>::to(&m.topic)
                    .payload(&payload)
                    .headers(headers);
                self.producer.send(record, queue).await
            }
        };
        let latency_us = started.elapsed().as_micros() as u64;
        match sent {
            Ok(_) => Ok(StepOutcome {
                latency_us,
                bytes_in: 0,
                bytes_out: (payload.len() + m.key.as_ref().map_or(0, String::len)) as u64,
                body: None,
                status: None,
            }),
            Err((e, _)) => Err(step_err(
                StepCode::KafkaProduce,
                format!("produce to `{}`: {e}", m.topic),
                Some(preview),
            )),
        }
    }

    pub async fn consume(&self, r: ConsumeRequest) -> Result<StepOutcome, StepError> {
        self.wait_match(r)
            .await
            .map_err(|e| mask_error(e, &self.secrets))
    }

    async fn wait_match(&self, r: ConsumeRequest) -> Result<StepOutcome, StepError> {
        let hub = self.hub.as_ref().ok_or_else(|| {
            step_err(
                StepCode::KafkaConsume,
                format!("consumer for `{}` is not running", r.topic),
                None,
            )
        })?;
        if !hub.topics.contains(&r.topic) {
            return Err(step_err(
                StepCode::KafkaConsume,
                format!("topic `{}` is not subscribed", r.topic),
                None,
            ));
        }
        let started = Instant::now();
        let deadline = tokio::time::Instant::now() + r.timeout;
        // Подписка до проверки буфера: сообщение, пришедшее между проверкой и ожиданием, не теряется.
        let mut seq = hub.seq.subscribe();
        loop {
            let taken = {
                let mut buffer = hub.buffer.lock().expect("consumer buffer");
                buffer.prune(Instant::now());
                buffer.take(&r.spec)
            };
            if let Some(m) = taken {
                return Ok(StepOutcome {
                    latency_us: started.elapsed().as_micros() as u64,
                    bytes_in: m.len,
                    bytes_out: 0,
                    body: m.body,
                    status: None,
                });
            }
            match tokio::time::timeout_at(deadline, seq.changed()).await {
                Ok(Ok(())) => continue,
                Ok(Err(_)) => {
                    return Err(step_err(StepCode::KafkaConsume, "consumer stopped", None));
                }
                Err(_) => {
                    return Err(step_err(
                        StepCode::Timeout,
                        format!(
                            "no message matched in `{}` within {} ms",
                            r.topic,
                            r.timeout.as_millis()
                        ),
                        Some(r.spec.describe()),
                    ));
                }
            }
        }
    }
}

impl Drop for KafkaExecutor {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

#[async_trait]
impl StepExecutor for KafkaExecutor {
    async fn execute(
        &self,
        step: &Step,
        ctx: &mut IterationCtx<'_>,
    ) -> Result<StepOutcome, StepError> {
        match &step.action {
            Action::KafkaProduce(p) => {
                let message =
                    prepare_produce(p, &*ctx).map_err(|e| mask_error(e, &self.secrets))?;
                self.produce(message).await
            }
            Action::KafkaConsume(c) => {
                let request =
                    prepare_consume(c, &*ctx).map_err(|e| mask_error(e, &self.secrets))?;
                let described = request.spec.describe();
                let out = self.consume(request).await?;
                check_message(step, &out, ctx).map_err(|e| {
                    let body = out.body.as_ref().map(|b| b.to_string());
                    mask_error(e.with_sample(Some(described), body), &self.secrets)
                })?;
                Ok(out)
            }
            Action::Http(h) => Err(step_err(
                StepCode::KafkaProduce,
                format!(
                    "http operation `{}` cannot run on the kafka executor",
                    h.operation
                ),
                None,
            )),
        }
    }

    async fn lag(&self) -> BTreeMap<String, i64> {
        self.hub
            .as_ref()
            .map(|h| h.lag.lock().expect("lag map").clone())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use glafira_load_model::suite::expr::Context;
    use glafira_load_model::suite::{ExistsTag, Expect, Matcher};
    use serde_json::json;

    use super::*;
    use crate::context::RunContext;
    use crate::errors::MAX_SAMPLE;

    /// Контекст без наборов данных: шаг `create` отдал `order_id`, секрет `API_TOKEN` задан.
    pub(crate) struct TestCtx;

    impl Context for TestCtx {
        fn data(&self, _set: &str, _col: &str) -> Option<serde_json::Value> {
            None
        }
        fn pick(&self, _set: &str, _col: &str) -> Option<serde_json::Value> {
            None
        }
        fn step(&self, step: &str, name: &str) -> Option<serde_json::Value> {
            (step == "create" && name == "order_id").then(|| json!("o-1"))
        }
        fn secret(&self, name: &str) -> Option<String> {
            (name == "API_TOKEN").then(|| "t0k".to_string())
        }
        fn env(&self, _key: &str) -> Option<String> {
            None
        }
        fn run_id(&self) -> &str {
            "r1"
        }
        fn agent_id(&self) -> &str {
            "a1"
        }
        fn random_u64(&self) -> u64 {
            7
        }
        fn now_unix_ms(&self) -> i64 {
            1_700_000_000_000
        }
    }

    fn received(
        topic: &str,
        key: Option<&str>,
        body: Option<serde_json::Value>,
        at: Instant,
    ) -> Received {
        Received {
            topic: topic.into(),
            key: key.map(str::to_string),
            len: 10,
            body,
            at,
        }
    }

    fn spec(topic: &str, key: Option<&str>, paths: &[(&str, &str)]) -> MatchSpec {
        MatchSpec {
            topic: topic.into(),
            key: key.map(str::to_string),
            jsonpath: paths
                .iter()
                .map(|(p, v)| (JsonPath::parse(p).unwrap(), v.to_string()))
                .collect(),
        }
    }

    #[test]
    fn topic_is_taken_from_operation_id() {
        assert_eq!(topic_of("kafka_produce:orders.paid"), Some("orders.paid"));
        assert_eq!(
            topic_of("kafka_consume:orders.created"),
            Some("orders.created")
        );
        assert_eq!(topic_of("kafka_consume:"), None);
        assert_eq!(topic_of("http:orders:POST /v1/orders"), None);
        assert_eq!(consumer_group("r1", "gen-1"), "glafira-load-r1-gen-1");
    }

    #[test]
    fn client_configs_follow_defaults_and_run() {
        let defaults = BTreeMap::from([
            ("acks".to_string(), "all".to_string()),
            ("message.timeout.ms".to_string(), "5000".to_string()),
        ]);
        let p = producer_config("k:9092", &defaults, "gen-1");
        assert_eq!(p.get("bootstrap.servers"), Some("k:9092"));
        assert_eq!(p.get("acks"), Some("all"));
        assert_eq!(
            p.get("message.timeout.ms"),
            Some("5000"),
            "значение из defaults не перекрывается"
        );
        assert_eq!(p.get("client.id"), Some("glafira-load-gen-1"));
        assert_eq!(
            producer_config("k:9092", &BTreeMap::new(), "a").get("message.timeout.ms"),
            Some("30000")
        );
        let c = consumer_config("k:9092", &defaults, "r1", "gen-1");
        assert_eq!(c.get("group.id"), Some("glafira-load-r1-gen-1"));
        assert_eq!(c.get("auto.offset.reset"), Some("latest"));
        assert_eq!(c.get("enable.auto.commit"), Some("false"));
    }

    #[test]
    fn buffer_evicts_by_size_and_age() {
        let t0 = Instant::now();
        let mut buf = MessageBuffer::new(3, Duration::from_secs(60));
        for i in 0..5 {
            buf.push(received("t", Some(&format!("k{i}")), None, t0));
        }
        assert_eq!(buf.len(), 3, "старые вытеснены");
        assert!(buf.take(&spec("t", Some("k0"), &[])).is_none());
        assert!(buf.take(&spec("t", Some("k4"), &[])).is_some());
        assert_eq!(buf.len(), 2, "совпавшее удалено");
        buf.push(received(
            "t",
            Some("new"),
            None,
            t0 + Duration::from_secs(120),
        ));
        assert_eq!(
            buf.len(),
            1,
            "сообщения старше 60 с выброшены при добавлении"
        );
        assert!(buf.take(&spec("t", Some("new"), &[])).is_some());
        assert!(buf.is_empty());
    }

    #[test]
    fn matching_by_key_jsonpath_and_topic() {
        let t0 = Instant::now();
        let body = json!({"orderId": "o-1", "qty": 2, "tags": ["a", "b"]});
        let m = received("orders.created", Some("o-1"), Some(body), t0);
        assert!(
            spec("orders.created", None, &[]).matches(&m),
            "без условий — первое сообщение топика"
        );
        assert!(spec("orders.created", Some("o-1"), &[]).matches(&m));
        assert!(!spec("orders.created", Some("o-2"), &[]).matches(&m));
        assert!(
            !spec("orders.paid", Some("o-1"), &[]).matches(&m),
            "чужой топик"
        );
        assert!(
            spec("orders.created", None, &[("$.qty", "2")]).matches(&m),
            "число сравнивается строкой"
        );
        assert!(
            spec("orders.created", None, &[("$.tags[*]", "b")]).matches(&m),
            "любой узел результата"
        );
        assert!(
            spec(
                "orders.created",
                Some("o-1"),
                &[("$.orderId", "o-1"), ("$.qty", "2")]
            )
            .matches(&m)
        );
        assert!(
            !spec(
                "orders.created",
                Some("o-1"),
                &[("$.orderId", "o-1"), ("$.qty", "3")]
            )
            .matches(&m),
            "все пути"
        );
        assert!(!spec("orders.created", None, &[("$.missing", "x")]).matches(&m));
        let no_json = received("orders.created", Some("o-1"), None, t0);
        assert!(spec("orders.created", Some("o-1"), &[]).matches(&no_json));
        assert!(
            !spec("orders.created", None, &[("$.qty", "2")]).matches(&no_json),
            "jsonpath без JSON-тела не совпадает"
        );
        assert_eq!(as_text(&json!("s")), "s");
        assert_eq!(as_text(&json!(2.5)), "2.5");
        assert_eq!(as_text(&json!({"a": 1})), r#"{"a":1}"#);
    }

    #[test]
    fn prepare_renders_key_value_headers_and_match() {
        let produce = ProduceStep {
            operation: "kafka_produce:orders.paid".into(),
            key: Some("{{steps.create.order_id}}".into()),
            value: Some(json!({"orderId": "{{steps.create.order_id}}", "by": "{{agent.id}}"})),
            value_text: None,
            headers: BTreeMap::from([(
                "Authorization".to_string(),
                "Bearer {{secret API_TOKEN}}".to_string(),
            )]),
            encoding: Encoding::Json,
        };
        let m = prepare_produce(&produce, &TestCtx).unwrap();
        assert_eq!(m.topic, "orders.paid");
        assert_eq!(m.key.as_deref(), Some("o-1"));
        assert_eq!(m.value, Some(json!({"orderId": "o-1", "by": "a1"})));
        assert_eq!(
            m.headers,
            vec![("Authorization".to_string(), "Bearer t0k".to_string())]
        );

        let missing = ProduceStep {
            key: Some("{{steps.nope.x}}".into()),
            ..produce.clone()
        };
        let err = prepare_produce(&missing, &TestCtx).unwrap_err();
        assert!(matches!(err.code, StepCode::ExtractMissing), "{err:?}");
        let bad_op = ProduceStep {
            operation: "http:x:GET /".into(),
            ..produce
        };
        assert!(matches!(
            prepare_produce(&bad_op, &TestCtx).unwrap_err().code,
            StepCode::KafkaProduce
        ));

        let consume = ConsumeStep {
            operation: "kafka_consume:orders.created".into(),
            match_: Match {
                key: Some("{{steps.create.order_id}}".into()),
                jsonpath: BTreeMap::from([(
                    "$.orderId".to_string(),
                    "{{steps.create.order_id}}".to_string(),
                )]),
            },
            timeout_ms: 1500,
        };
        let r = prepare_consume(&consume, &TestCtx).unwrap();
        assert_eq!(
            (r.topic.as_str(), r.timeout),
            ("orders.created", Duration::from_millis(1500))
        );
        assert_eq!(r.spec.key.as_deref(), Some("o-1"));
        assert_eq!(r.spec.jsonpath.len(), 1);
        assert_eq!(
            r.spec.describe(),
            "topic=orders.created key=o-1 $.orderId=o-1"
        );
        let bad_path = ConsumeStep {
            match_: Match {
                key: None,
                jsonpath: BTreeMap::from([("$[".to_string(), "1".to_string())]),
            },
            ..consume
        };
        assert!(matches!(
            prepare_consume(&bad_path, &TestCtx).unwrap_err().code,
            StepCode::KafkaConsume
        ));
    }

    #[test]
    fn errors_are_masked_in_raw_and_json_forms() {
        let forms = secret_forms(&[
            "t0k".to_string(),
            "a\"b".to_string(),
            String::new(),
            "t0k".to_string(),
        ]);
        assert_eq!(
            forms,
            ["t0k".to_string(), "a\"b".to_string(), r#"a\"b"#.to_string()]
        );
        let e = StepError {
            code: StepCode::KafkaProduce,
            message: "produce to `t`: t0k".into(),
            request: Some(r#"{"p":"a\"b","t":"t0k"}"#.into()),
            response: Some("t0k".into()),
        };
        let m = mask_error(e, &forms);
        assert_eq!(m.message, "produce to `t`: ***");
        assert_eq!(m.request.as_deref(), Some(r#"{"p":"***","t":"***"}"#));
        assert_eq!(m.response.as_deref(), Some("***"));
        let long = StepError {
            request: Some("x".repeat(MAX_SAMPLE * 2)),
            ..m
        };
        assert!(
            mask_error(long, &forms).request.unwrap().len() <= MAX_SAMPLE,
            "пример обрезан"
        );
    }

    #[test]
    fn consumed_message_is_checked_and_extracted() {
        let run = RunContext::default();
        let mut ctx = IterationCtx::new(&run, 0);
        ctx.begin_step("wait");
        let step = Step {
            id: Some("wait".into()),
            action: Action::KafkaConsume(ConsumeStep {
                operation: "kafka_consume:orders.created".into(),
                match_: Match::default(),
                timeout_ms: 1000,
            }),
            expect: Expect {
                jsonpath: BTreeMap::from([(
                    "$.qty".to_string(),
                    Matcher::Exists(ExistsTag::Exists),
                )]),
                max_ms: Some(5),
                ..Expect::default()
            },
            extract: BTreeMap::from([("qty".to_string(), "$.qty".to_string())]),
        };
        let out = StepOutcome {
            latency_us: 2_000,
            body: Some(json!({"qty": 2})),
            ..StepOutcome::default()
        };
        check_message(&step, &out, &mut ctx).unwrap();
        assert_eq!(
            ctx.step("wait", "qty"),
            Some(json!(2)),
            "extract по значению сообщения"
        );

        let slow = StepOutcome {
            latency_us: 6_000,
            ..out.clone()
        };
        assert!(matches!(
            check_message(&step, &slow, &mut ctx).unwrap_err().code,
            StepCode::ExpectMaxMs
        ));
        let other = StepOutcome {
            body: Some(json!({"x": 1})),
            ..out.clone()
        };
        assert!(matches!(
            check_message(&step, &other, &mut ctx).unwrap_err().code,
            StepCode::ExpectJsonpath
        ));
        let no_expect = Step {
            expect: Expect::default(),
            ..step
        };
        let not_json = StepOutcome { body: None, ..out };
        assert!(matches!(
            check_message(&no_expect, &not_json, &mut ctx)
                .unwrap_err()
                .code,
            StepCode::ExtractMissing
        ));
    }
}
