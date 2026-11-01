//! Kafka против Redpanda в Docker (testcontainers). Без Docker пропускаются (`#[ignore]`):
//!
//!     cargo test -p glafira-load-engine --test kafka -- --ignored
//!
//! В CI (job `linux`) запускаются той же командой. Адрес брокера объявляется заранее через
//! свободный порт хоста, потому что Redpanda должна знать `advertise-kafka-addr` при старте.

use std::collections::BTreeMap;
use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use axum::extract::Path;
use axum::routing::get;
use axum::{Json, Router};
use glafira_load_engine::avro;
use glafira_load_engine::errors::StepCode;
use glafira_load_engine::executor::StepExecutor;
use glafira_load_engine::kafka::{KafkaExecutor, prepare_consume, prepare_produce};
use glafira_load_model::suite::expr::Context;
use glafira_load_model::suite::{ConsumeStep, Encoding, Match, ProduceStep};
use rdkafka::ClientConfig;
use rdkafka::admin::{AdminClient, AdminOptions, NewTopic, TopicReplication};
use rdkafka::client::DefaultClientContext;
use rdkafka::consumer::{BaseConsumer, Consumer};
use rdkafka::message::Message;
use serde_json::json;
use testcontainers::core::{ContainerPort, WaitFor};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, GenericImage, ImageExt};

const SCHEMA: &str = r#"{"type":"record","name":"OrderPaid","namespace":"orders","fields":[
    {"name":"orderId","type":"string"},
    {"name":"amount","type":"long"},
    {"name":"note","type":["null","string"],"default":null}]}"#;

struct TestCtx;

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
    fn secret(&self, _name: &str) -> Option<String> {
        None
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

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

async fn redpanda() -> (ContainerAsync<GenericImage>, String) {
    let port = free_port();
    let cmd: Vec<String> = [
        "redpanda",
        "start",
        "--mode",
        "dev-container",
        "--smp",
        "1",
        "--memory",
        "512M",
        "--overprovisioned",
        "--kafka-addr",
        "PLAINTEXT://0.0.0.0:9092",
    ]
    .iter()
    .map(|s| s.to_string())
    .chain([format!(
        "--advertise-kafka-addr=PLAINTEXT://127.0.0.1:{port}"
    )])
    .collect();
    let container = GenericImage::new("redpandadata/redpanda", "v24.2.7")
        .with_exposed_port(ContainerPort::Tcp(9092))
        .with_wait_for(WaitFor::message_on_stderr("Successfully started Redpanda!"))
        .with_mapped_port(port, ContainerPort::Tcp(9092))
        .with_cmd(cmd)
        .start()
        .await
        .expect("docker with redpandadata/redpanda");
    (container, format!("127.0.0.1:{port}"))
}

async fn create_topics(bootstrap: &str, topics: &[&str]) {
    let admin: AdminClient<DefaultClientContext> = ClientConfig::new()
        .set("bootstrap.servers", bootstrap)
        .create()
        .unwrap();
    let specs: Vec<NewTopic> = topics
        .iter()
        .map(|t| NewTopic::new(t, 1, TopicReplication::Fixed(1)))
        .collect();
    for result in admin
        .create_topics(&specs, &AdminOptions::new())
        .await
        .unwrap()
    {
        result.unwrap();
    }
}

fn produce_step(operation: &str, encoding: Encoding, value: serde_json::Value) -> ProduceStep {
    ProduceStep {
        operation: operation.into(),
        key: Some("{{steps.create.order_id}}".into()),
        value: Some(value),
        value_text: None,
        headers: BTreeMap::from([("source".to_string(), "glafira-load".to_string())]),
        encoding,
    }
}

fn consume_step(key: Option<&str>, paths: &[(&str, &str)], timeout_ms: u64) -> ConsumeStep {
    ConsumeStep {
        operation: "kafka_consume:orders.created".into(),
        match_: Match {
            key: key.map(str::to_string),
            jsonpath: paths
                .iter()
                .map(|(p, v)| (p.to_string(), v.to_string()))
                .collect(),
        },
        timeout_ms,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore]
async fn produce_json_then_consume_by_key_and_jsonpath() {
    let (_rp, bootstrap) = redpanda().await;
    create_topics(&bootstrap, &["orders.created"]).await;
    let ex = KafkaExecutor::new(
        &bootstrap,
        &BTreeMap::new(),
        &["orders.created".to_string()],
        "r1",
        "a1",
        None,
        &BTreeMap::new(),
    )
    .await
    .unwrap();

    let step = produce_step(
        "kafka_produce:orders.created",
        Encoding::Json,
        json!({"orderId": "{{steps.create.order_id}}", "qty": 2}),
    );
    let out = ex
        .produce(prepare_produce(&step, &TestCtx).unwrap())
        .await
        .unwrap();
    assert!(out.bytes_out > 0 && out.body.is_none() && out.status.is_none());

    let got = ex
        .consume(
            prepare_consume(
                &consume_step(Some("{{steps.create.order_id}}"), &[("$.qty", "2")], 10_000),
                &TestCtx,
            )
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(got.body, Some(json!({"orderId": "o-1", "qty": 2})));
    assert!(got.bytes_in > 0);

    // Совпавшее сообщение изъято из буфера: повторное ожидание того же ключа истекает.
    let again = ex
        .consume(prepare_consume(&consume_step(Some("o-1"), &[], 1_500), &TestCtx).unwrap())
        .await;
    match again {
        Err(e) => assert!(matches!(e.code, StepCode::Timeout), "{e:?}"),
        Ok(_) => panic!("message must be consumed once"),
    }
    ex.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore]
async fn produce_avro_with_schema_registry_stub() {
    let (_rp, bootstrap) = redpanda().await;
    create_topics(&bootstrap, &["orders.paid"]).await;
    let hits = Arc::new(AtomicUsize::new(0));
    let counter = hits.clone();
    let app = Router::new().route(
        "/subjects/{subject}/versions/latest",
        get(move |Path(subject): Path<String>| {
            let counter = counter.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                Json(json!({"id": 7, "version": 1, "subject": subject, "schema": SCHEMA}))
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let registry = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let schemas = BTreeMap::from([("kafka_produce:orders.paid".to_string(), SCHEMA.to_string())]);
    let ex = KafkaExecutor::new(
        &bootstrap,
        &BTreeMap::new(),
        &[],
        "r1",
        "a1",
        Some(registry),
        &schemas,
    )
    .await
    .unwrap();
    let step = produce_step(
        "kafka_produce:orders.paid",
        Encoding::Avro,
        json!({"orderId": "{{steps.create.order_id}}", "amount": 250}),
    );
    ex.produce(prepare_produce(&step, &TestCtx).unwrap())
        .await
        .unwrap();
    ex.produce(prepare_produce(&step, &TestCtx).unwrap())
        .await
        .unwrap();
    assert_eq!(
        hits.load(Ordering::SeqCst),
        1,
        "schema id is requested once per run"
    );

    // Сырое сообщение читается обычным потребителем с начала топика.
    let reader: BaseConsumer = ClientConfig::new()
        .set("bootstrap.servers", &bootstrap)
        .set("group.id", "test-reader")
        .set("auto.offset.reset", "earliest")
        .create()
        .unwrap();
    reader.subscribe(&["orders.paid"]).unwrap();
    let started = Instant::now();
    let message = loop {
        if let Some(m) = reader.poll(Duration::from_secs(1)) {
            break m.unwrap().detach();
        }
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "no message within 30 s"
        );
    };
    let (id, body) = avro::unframe(message.payload().unwrap()).unwrap();
    assert_eq!(id, 7);
    let schema = apache_avro::Schema::parse_str(SCHEMA).unwrap();
    assert_eq!(
        avro::decode(&schema, body).unwrap(),
        json!({"orderId": "o-1", "amount": 250, "note": null})
    );
    assert_eq!(message.key(), Some(b"o-1".as_slice()));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore]
async fn consume_times_out_without_matching_message() {
    let (_rp, bootstrap) = redpanda().await;
    create_topics(&bootstrap, &["orders.created"]).await;
    let ex = KafkaExecutor::new(
        &bootstrap,
        &BTreeMap::new(),
        &["orders.created".to_string()],
        "r1",
        "a1",
        None,
        &BTreeMap::new(),
    )
    .await
    .unwrap();
    let step = produce_step(
        "kafka_produce:orders.created",
        Encoding::Json,
        json!({"orderId": "other"}),
    );
    ex.produce(prepare_produce(&step, &TestCtx).unwrap())
        .await
        .unwrap();
    let started = Instant::now();
    let err = ex
        .consume(
            prepare_consume(
                &consume_step(None, &[("$.orderId", "nope")], 1_000),
                &TestCtx,
            )
            .unwrap(),
        )
        .await
        .expect_err("timeout");
    assert!(matches!(err.code, StepCode::Timeout), "{err:?}");
    assert!(started.elapsed() >= Duration::from_millis(1_000));
    assert_eq!(
        err.request.as_deref(),
        Some("topic=orders.created $.orderId=nope")
    );
    assert!(err.message.contains("1000 ms"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore]
async fn lag_is_reported_per_consume_operation() {
    let (_rp, bootstrap) = redpanda().await;
    create_topics(&bootstrap, &["orders.created"]).await;
    let ex = KafkaExecutor::new(
        &bootstrap,
        &BTreeMap::new(),
        &["orders.created".to_string()],
        "r1",
        "a1",
        None,
        &BTreeMap::new(),
    )
    .await
    .unwrap();
    for i in 0..3 {
        let step = produce_step(
            "kafka_produce:orders.created",
            Encoding::Json,
            json!({"i": i}),
        );
        ex.produce(prepare_produce(&step, &TestCtx).unwrap())
            .await
            .unwrap();
    }
    tokio::time::sleep(Duration::from_millis(2_500)).await;
    let lag = ex.lag().await;
    let value = lag
        .get("kafka_consume:orders.created")
        .copied()
        .expect("lag of the consume operation");
    assert!(value >= 0, "{lag:?}");
}
