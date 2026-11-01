//! Шаги `kafka_produce` и `kafka_consume` на rdkafka (спека L2 §5.2, решение плана 6).
//!
//! Продюсер один на агента. Потребитель один на все `kafka_consume`-топики сьюта: группа
//! `glafira-load-<run_id>-<agent_id>`, чтение с конца, фоновая задача складывает сообщения в буфер
//! (не больше `BUFFER_MAX` и не старше `BUFFER_TTL`), шаг ждёт в буфере совпадение по `match`.
//! Лаг снимается раз в секунду по смещениям. Рендер шаблонов — до первого `await`.
//! `expect` и `extract` шага `kafka_consume` применяются к значению сообщения (спека §3.4),
//! тексты ошибок и примеры маскируются значениями секретов прогона.
//!
//! Каждое сообщение в буфере получает возрастающий номер `seq`: ожидающий шаг при каждом
//! пробуждении просматривает только сообщения новее последнего просмотренного, а не весь буфер.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use glafira_load_model::schema::SchemaSummary;
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
    // Кооперативный протокол: при поздних ребалансах в `rebalance` приходят только добавленные
    // партиции, и на high watermark переставляются лишь они; при eager-протоколе каждая
    // ребалансировка переставляла бы все партиции и теряла сообщения между ними.
    if !defaults.contains_key("partition.assignment.strategy") {
        c.set("partition.assignment.strategy", "cooperative-sticky");
    }
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
/// статуса у сообщения нет, задержка — ожидание совпадения. `response_schema` — схема
/// сообщения операции из реестра (для `expect.schema: response`).
pub fn check_message(
    step: &Step,
    out: &StepOutcome,
    ctx: &mut IterationCtx<'_>,
    response_schema: Option<&SchemaSummary>,
) -> Result<(), StepError> {
    check_expect(
        &step.expect,
        None,
        out.body.as_ref(),
        out.latency_us as f64 / 1000.0,
        response_schema,
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
    /// Номер в порядке поступления; присваивает `MessageBuffer::push` (начиная с 1).
    pub seq: u64,
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
            seq: 0,
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

/// Буфер последних сообщений: по размеру и по возрасту. Сообщения лежат в порядке
/// поступления, их `seq` строго возрастает от начала к концу.
pub struct MessageBuffer {
    items: VecDeque<Received>,
    max: usize,
    ttl: Duration,
    next_seq: u64,
}

impl MessageBuffer {
    pub fn new(max: usize, ttl: Duration) -> Self {
        Self {
            items: VecDeque::new(),
            max,
            ttl,
            next_seq: 1,
        }
    }

    /// Добавляет сообщение, присваивает ему следующий `seq` и возвращает этот номер.
    pub fn push(&mut self, mut m: Received) -> u64 {
        self.prune(m.at);
        if self.items.len() >= self.max {
            self.items.pop_front();
        }
        let seq = self.next_seq;
        self.next_seq += 1;
        m.seq = seq;
        self.items.push_back(m);
        seq
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

    /// Первое совпавшее сообщение во всём буфере; оно удаляется из буфера.
    pub fn take(&mut self, spec: &MatchSpec) -> Option<Received> {
        self.take_since(spec, 0).0
    }

    /// Первое совпавшее сообщение среди тех, у кого `seq > after` (0 — весь буфер); оно
    /// удаляется из буфера. Второе значение — наибольший просмотренный `seq`: его ожидающий
    /// шаг передаёт как `after` при следующем пробуждении, чтобы не проверять старое повторно.
    pub fn take_since(&mut self, spec: &MatchSpec, after: u64) -> (Option<Received>, u64) {
        self.take_since_by(after, |m| spec.matches(m))
    }

    /// `take_since` с произвольным условием; стоимость — O(число сообщений новее `after`).
    fn take_since_by(
        &mut self,
        after: u64,
        mut matches: impl FnMut(&Received) -> bool,
    ) -> (Option<Received>, u64) {
        let last = self.items.back().map_or(after, |m| m.seq.max(after));
        // Буфер упорядочен по `seq`: граница ищется с конца и стоит O(новых сообщений).
        let start = self
            .items
            .iter()
            .rposition(|m| m.seq <= after)
            .map_or(0, |i| i + 1);
        let found = (start..self.items.len()).find(|&i| matches(&self.items[i]));
        (found.and_then(|i| self.items.remove(i)), last)
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
    /// `seq` последнего принятого сообщения: будит ожидающие шаги.
    seq: watch::Sender<u64>,
    /// Операция `kafka_consume:<topic>` → лаг, обновляется раз в секунду.
    lag: Mutex<BTreeMap<String, i64>>,
    /// Остановка прогона: гасит фоновые задачи и будит ожидающие шаги.
    cancel: CancellationToken,
}

impl ConsumerHub {
    fn new(consumer: HubConsumer, topics: &[String], cancel: CancellationToken) -> Self {
        let (seq, _) = watch::channel(0u64);
        Self {
            consumer,
            topics: topics.to_vec(),
            buffer: Mutex::new(MessageBuffer::new(BUFFER_MAX, BUFFER_TTL)),
            seq,
            lag: Mutex::new(BTreeMap::new()),
            cancel,
        }
    }

    /// Кладёт сообщение в буфер и будит ожидающие шаги.
    fn accept(&self, m: Received) {
        let seq = self.buffer.lock().expect("consumer buffer").push(m);
        self.seq.send_replace(seq);
    }
}

async fn read_loop(hub: Arc<ConsumerHub>) {
    loop {
        tokio::select! {
            _ = hub.cancel.cancelled() => break,
            next = hub.consumer.recv() => match next {
                Ok(m) => hub.accept(Received::from_message(&m)),
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
    /// Id операции `kafka_consume` → схема сообщения из реестра (`expect.schema: response`).
    schemas: BTreeMap<String, SchemaSummary>,
    cancel: CancellationToken,
}

impl KafkaExecutor {
    /// `consume_topics` — топики всех `kafka_consume` сьюта (пусто — потребитель не создаётся);
    /// `avro_schemas` — id операции `kafka_produce` с `encoding: avro` → текст `.avsc`.
    /// Возвращает после назначения партиций (до `ASSIGNMENT_WAIT`), чтобы не терять первые сообщения.
    ///
    /// В отличие от `HttpExecutor::new`, секреты сюда не передаются: фабрика исполнителей
    /// ОБЯЗАНА вызвать `with_secrets` (иначе ошибки и примеры не маскируются) и
    /// `with_response_schemas` (иначе `expect.schema: response` на `kafka_consume` всегда падает).
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
            let hub = Arc::new(ConsumerHub::new(consumer, consume_topics, cancel.clone()));
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
            schemas: BTreeMap::new(),
            cancel,
        })
    }

    /// Значения секретов прогона: ими маскируются тексты ошибок и примеры. Без этого вызова
    /// маскирования нет — фабрика исполнителей обязана его сделать (см. `new`).
    pub fn with_secrets(mut self, secrets: &[String]) -> Self {
        self.secrets = secret_forms(secrets);
        self
    }

    /// Схемы сообщений операций `kafka_consume` из реестра (id операции → схема, как у
    /// `HttpExecutor::with_schemas`) — для `expect.schema: response`.
    pub fn with_response_schemas(mut self, schemas: BTreeMap<String, SchemaSummary>) -> Self {
        self.schemas = schemas;
        self
    }

    /// Останавливает фоновые задачи потребителя и будит ожидающие шаги `kafka_consume`.
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
        let mut wake = hub.seq.subscribe();
        // Наибольший просмотренный `seq`: первый проход — весь буфер, дальше только новое.
        let mut seen = 0u64;
        loop {
            let (taken, last) = {
                let mut buffer = hub.buffer.lock().expect("consumer buffer");
                buffer.prune(Instant::now());
                buffer.take_since(&r.spec, seen)
            };
            seen = last;
            if let Some(m) = taken {
                return Ok(StepOutcome {
                    latency_us: started.elapsed().as_micros() as u64,
                    bytes_in: m.len,
                    bytes_out: 0,
                    body: m.body,
                    status: None,
                });
            }
            let changed = tokio::select! {
                biased;
                _ = hub.cancel.cancelled() => {
                    return Err(step_err(StepCode::KafkaConsume, "consumer stopped", None));
                }
                changed = tokio::time::timeout_at(deadline, wake.changed()) => changed,
            };
            match changed {
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
                let schema = self.schemas.get(&c.operation);
                check_message(step, &out, ctx, schema).map_err(|e| {
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
    use glafira_load_model::suite::{ExistsTag, Expect, Matcher, SchemaExpect};
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
            seq: 0,
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
        assert_eq!(
            c.get("partition.assignment.strategy"),
            Some("cooperative-sticky"),
            "кооперативный ребаланс по умолчанию"
        );
        let own = BTreeMap::from([(
            "partition.assignment.strategy".to_string(),
            "range".to_string(),
        )]);
        assert_eq!(
            consumer_config("k:9092", &own, "r1", "gen-1").get("partition.assignment.strategy"),
            Some("range"),
            "стратегия из defaults.kafka не перекрывается"
        );
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
        check_message(&step, &out, &mut ctx, None).unwrap();
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
            check_message(&step, &slow, &mut ctx, None)
                .unwrap_err()
                .code,
            StepCode::ExpectMaxMs
        ));
        let other = StepOutcome {
            body: Some(json!({"x": 1})),
            ..out.clone()
        };
        assert!(matches!(
            check_message(&step, &other, &mut ctx, None)
                .unwrap_err()
                .code,
            StepCode::ExpectJsonpath
        ));
        let no_expect = Step {
            expect: Expect::default(),
            ..step
        };
        let not_json = StepOutcome { body: None, ..out };
        assert!(matches!(
            check_message(&no_expect, &not_json, &mut ctx, None)
                .unwrap_err()
                .code,
            StepCode::ExtractMissing
        ));
    }

    #[test]
    fn buffer_scans_only_messages_after_seen_seq() {
        let t0 = Instant::now();
        let mut buf = MessageBuffer::new(100, Duration::from_secs(60));
        let seqs: Vec<u64> = (0..5)
            .map(|i| buf.push(received("t", Some(&format!("k{i}")), None, t0)))
            .collect();
        assert_eq!(seqs, [1, 2, 3, 4, 5], "seq возрастает с 1");

        // Первый проход — весь буфер.
        let mut evaluated = Vec::new();
        let (none, last) = buf.take_since_by(0, |m| {
            evaluated.push(m.seq);
            false
        });
        assert!(none.is_none());
        assert_eq!((evaluated.clone(), last), (vec![1, 2, 3, 4, 5], 5));

        // Пробуждение без новых сообщений не проверяет ничего.
        evaluated.clear();
        let (_, last) = buf.take_since_by(last, |m| {
            evaluated.push(m.seq);
            false
        });
        assert!(
            evaluated.is_empty(),
            "старые сообщения не проверяются повторно"
        );
        assert_eq!(last, 5);

        // Пришли три новых — проверяются только они.
        for i in 5..8 {
            buf.push(received("t", Some(&format!("k{i}")), None, t0));
        }
        let (_, last) = buf.take_since_by(last, |m| {
            evaluated.push(m.seq);
            false
        });
        assert_eq!((evaluated.clone(), last), (vec![6, 7, 8], 8));

        // Совпадение ищется только среди новых: старый k1 не берётся, новый k6 — берётся.
        assert!(buf.take_since(&spec("t", Some("k1"), &[]), 5).0.is_none());
        let (got, last) = buf.take_since(&spec("t", Some("k6"), &[]), 5);
        assert_eq!((got.map(|m| m.seq), last), (Some(7), 8));
        assert_eq!(buf.len(), 7, "совпавшее удалено");
        // После изъятия из середины граница по-прежнему находится верно.
        evaluated.clear();
        let (_, last) = buf.take_since_by(6, |m| {
            evaluated.push(m.seq);
            false
        });
        assert_eq!((evaluated, last), (vec![8], 8));
        assert!(
            buf.take(&spec("t", Some("k1"), &[])).is_some(),
            "take видит весь буфер"
        );
    }

    /// Исполнитель без брокера: клиенты создаются, но никуда не подключаются; фоновых задач нет,
    /// сообщения кладутся в буфер вручную (`ConsumerHub::accept`).
    fn offline(topics: &[&str]) -> KafkaExecutor {
        let none = BTreeMap::new();
        let producer: FutureProducer = producer_config("127.0.0.1:1", &none, "a1")
            .create()
            .unwrap();
        let cancel = CancellationToken::new();
        let hub = (!topics.is_empty()).then(|| {
            let consumer: HubConsumer = consumer_config("127.0.0.1:1", &none, "r1", "a1")
                .create_with_context(StartAtEnd)
                .unwrap();
            let topics: Vec<String> = topics.iter().map(|t| t.to_string()).collect();
            Arc::new(ConsumerHub::new(consumer, &topics, cancel.clone()))
        });
        KafkaExecutor {
            producer,
            hub,
            avro: AvroCodec::new(None, &BTreeMap::new()).unwrap(),
            secrets: Vec::new(),
            schemas: BTreeMap::new(),
            cancel,
        }
    }

    fn consume_request(topic: &str, key: &str, timeout_ms: u64) -> ConsumeRequest {
        ConsumeRequest {
            topic: topic.into(),
            spec: spec(topic, Some(key), &[]),
            timeout: Duration::from_millis(timeout_ms),
        }
    }

    #[tokio::test]
    async fn waiting_consume_gets_a_later_message_and_shutdown_wakes_waiters() {
        let ex = Arc::new(offline(&["orders.created"]));
        let hub = ex.hub.clone().unwrap();
        hub.accept(received(
            "orders.created",
            Some("old"),
            None,
            Instant::now(),
        ));

        let waiter = {
            let ex = ex.clone();
            tokio::spawn(async move {
                ex.consume(consume_request("orders.created", "o-1", 10_000))
                    .await
            })
        };
        tokio::time::sleep(Duration::from_millis(50)).await;
        hub.accept(received(
            "orders.created",
            Some("o-1"),
            Some(json!({"qty": 2})),
            Instant::now(),
        ));
        let out = tokio::time::timeout(Duration::from_secs(5), waiter)
            .await
            .expect("waiter woke up")
            .unwrap()
            .unwrap();
        assert_eq!(out.body, Some(json!({"qty": 2})));

        // После shutdown ожидание завершается сразу, а не по таймауту шага.
        let waiter = {
            let ex = ex.clone();
            tokio::spawn(async move {
                ex.consume(consume_request("orders.created", "o-2", 60_000))
                    .await
            })
        };
        tokio::time::sleep(Duration::from_millis(50)).await;
        let started = Instant::now();
        ex.shutdown();
        let err = tokio::time::timeout(Duration::from_secs(5), waiter)
            .await
            .expect("shutdown wakes the waiter")
            .unwrap()
            .unwrap_err();
        assert_eq!(
            (err.code, err.message.as_str()),
            (StepCode::KafkaConsume, "consumer stopped")
        );
        assert!(started.elapsed() < Duration::from_secs(5));
        // Новые ожидания после shutdown тоже не ждут.
        let err = ex
            .consume(consume_request("orders.created", "o-3", 60_000))
            .await
            .unwrap_err();
        assert_eq!(err.message, "consumer stopped");
    }

    fn consume_step_with_schema() -> Step {
        Step {
            id: Some("wait".into()),
            action: Action::KafkaConsume(ConsumeStep {
                operation: "kafka_consume:orders.created".into(),
                match_: Match {
                    key: Some("o-1".into()),
                    jsonpath: BTreeMap::new(),
                },
                timeout_ms: 1_000,
            }),
            expect: Expect {
                schema: Some(SchemaExpect::Response),
                ..Expect::default()
            },
            extract: BTreeMap::new(),
        }
    }

    #[test]
    fn check_message_validates_the_response_schema() {
        let run = RunContext::default();
        let mut ctx = IterationCtx::new(&run, 0);
        ctx.begin_step("wait");
        let step = consume_step_with_schema();
        let out = StepOutcome {
            body: Some(json!({"orderId": "o-1"})),
            ..StepOutcome::default()
        };
        let good = SchemaSummary::object(
            "body",
            vec![SchemaSummary::leaf("orderId", "string").required(true)],
        );
        let strict = SchemaSummary::object(
            "body",
            vec![SchemaSummary::leaf("missing", "string").required(true)],
        );
        check_message(&step, &out, &mut ctx, Some(&good)).unwrap();
        assert_eq!(
            check_message(&step, &out, &mut ctx, Some(&strict))
                .unwrap_err()
                .code,
            StepCode::ExpectSchema
        );
        assert_eq!(
            check_message(&step, &out, &mut ctx, None).unwrap_err().code,
            StepCode::ExpectSchema,
            "без схемы в реестре проверка не проходит"
        );
    }

    #[tokio::test]
    async fn consume_step_uses_response_schemas_by_operation() {
        let good = SchemaSummary::object(
            "body",
            vec![SchemaSummary::leaf("orderId", "string").required(true)],
        );
        let strict = SchemaSummary::object(
            "body",
            vec![SchemaSummary::leaf("missing", "string").required(true)],
        );
        let step = consume_step_with_schema();
        let run = RunContext::default();
        for (schema, passes) in [(good, true), (strict, false)] {
            let ex = offline(&["orders.created"]).with_response_schemas(BTreeMap::from([(
                "kafka_consume:orders.created".to_string(),
                schema,
            )]));
            ex.hub.as_ref().unwrap().accept(received(
                "orders.created",
                Some("o-1"),
                Some(json!({"orderId": "o-1"})),
                Instant::now(),
            ));
            let mut ctx = IterationCtx::new(&run, 0);
            ctx.begin_step("wait");
            let result = ex.execute(&step, &mut ctx).await;
            if passes {
                assert_eq!(result.unwrap().body, Some(json!({"orderId": "o-1"})));
            } else {
                assert_eq!(result.unwrap_err().code, StepCode::ExpectSchema);
            }
        }
    }

    #[tokio::test]
    async fn execute_masks_secrets_after_with_secrets() {
        let mut run = RunContext::default();
        run.secrets
            .insert("API_TOKEN".to_string(), "s3cr3t".to_string());
        let secrets = run.secret_values();
        let produce = |key: &str, value_text: &str, encoding| Step {
            id: Some("send".into()),
            action: Action::KafkaProduce(ProduceStep {
                operation: "kafka_produce:orders.paid".into(),
                key: Some(key.into()),
                value: None,
                value_text: Some(value_text.into()),
                headers: BTreeMap::new(),
                encoding,
            }),
            expect: Expect::default(),
            extract: BTreeMap::new(),
        };
        // Ошибка рендера: текст выражения содержит значение секрета.
        let render = produce("{{steps.s3cr3t.x}}", "{}", Encoding::Json);
        // Ошибка до отправки: пример — отрендеренный `value_text` с секретом.
        let avro = produce("k", "token {{secret API_TOKEN}}", Encoding::Avro);

        let ex = offline(&[]);
        let mut ctx = IterationCtx::new(&run, 0);
        ctx.begin_step("send");
        let raw = ex.execute(&render, &mut ctx).await.unwrap_err();
        assert!(
            raw.message.contains("s3cr3t"),
            "без with_secrets маскирования нет: {raw:?}"
        );

        let ex = offline(&[]).with_secrets(&secrets);
        let e = ex.execute(&render, &mut ctx).await.unwrap_err();
        assert_eq!(e.code, StepCode::ExtractMissing);
        assert!(
            !e.message.contains("s3cr3t") && e.message.contains("***"),
            "{e:?}"
        );
        let e = ex.execute(&avro, &mut ctx).await.unwrap_err();
        assert_eq!(e.code, StepCode::AvroEncode);
        let request = e.request.unwrap();
        assert!(
            !request.contains("s3cr3t") && request.contains("token ***"),
            "{request}"
        );
    }
}
