//! Исполнитель шага: результат, трейт и диспетчер по виду действия (спека L2 §5.2).

use std::collections::BTreeMap;

use async_trait::async_trait;
use glafira_load_model::suite::{Action, Step};
use serde_json::Value;

use crate::context::IterationCtx;
use crate::errors::{StepCode, StepError};
use crate::http::HttpExecutor;

/// Удачный шаг: задержка, байты, разобранное тело (если JSON) и статус (у HTTP).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct StepOutcome {
    pub latency_us: u64,
    pub bytes_in: u64,
    pub bytes_out: u64,
    pub body: Option<Value>,
    pub status: Option<u16>,
}

/// Исполняет один шаг: рендерит выражения из `ctx`, проверяет `expect`, кладёт `extract`
/// в `ctx` под текущим шагом (`IterationCtx::begin_step`).
#[async_trait]
pub trait StepExecutor: Send + Sync {
    async fn execute(
        &self,
        step: &Step,
        ctx: &mut IterationCtx<'_>,
    ) -> Result<StepOutcome, StepError>;

    /// Лаг потребителя по операциям `kafka_consume`; у HTTP пусто.
    async fn lag(&self) -> BTreeMap<String, i64>;
}

/// Диспетчер: HTTP — всегда, Kafka — если настроен (Task 7 даёт `KafkaExecutor`,
/// реализующий `StepExecutor`; он кладётся сюда в `Box`).
pub struct Executors {
    pub http: HttpExecutor,
    pub kafka: Option<Box<dyn StepExecutor>>,
}

#[async_trait]
impl StepExecutor for Executors {
    async fn execute(
        &self,
        step: &Step,
        ctx: &mut IterationCtx<'_>,
    ) -> Result<StepOutcome, StepError> {
        match &step.action {
            Action::Http(_) => self.http.execute(step, ctx).await,
            Action::KafkaProduce(_) | Action::KafkaConsume(_) => match &self.kafka {
                Some(k) => k.execute(step, ctx).await,
                None => {
                    let code = match &step.action {
                        Action::KafkaProduce(_) => StepCode::KafkaProduce,
                        _ => StepCode::KafkaConsume,
                    };
                    Err(StepError::new(code, "kafka not configured"))
                }
            },
        }
    }

    async fn lag(&self) -> BTreeMap<String, i64> {
        match &self.kafka {
            Some(k) => k.lag().await,
            None => BTreeMap::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::RunContext;
    use glafira_load_model::suite::{ConsumeStep, Expect, HttpDefaults, Match, ProduceStep};

    fn step(action: Action) -> Step {
        Step {
            id: None,
            action,
            expect: Expect::default(),
            extract: BTreeMap::new(),
        }
    }

    #[tokio::test]
    async fn kafka_steps_fail_without_a_kafka_executor() {
        let ex = Executors {
            http: HttpExecutor::new("http://127.0.0.1:1", &HttpDefaults::default(), &[]).unwrap(),
            kafka: None,
        };
        let run = RunContext::default();
        let mut ctx = IterationCtx::new(&run, 0);
        let produce = step(Action::KafkaProduce(ProduceStep {
            operation: "kafka_produce:orders.paid".into(),
            key: None,
            value: None,
            value_text: None,
            headers: BTreeMap::new(),
            encoding: Default::default(),
        }));
        let e = ex.execute(&produce, &mut ctx).await.unwrap_err();
        assert_eq!(
            (e.code, e.message.as_str()),
            (StepCode::KafkaProduce, "kafka not configured")
        );
        let consume = step(Action::KafkaConsume(ConsumeStep {
            operation: "kafka_consume:orders.created".into(),
            match_: Match::default(),
            timeout_ms: 10,
        }));
        let e = ex.execute(&consume, &mut ctx).await.unwrap_err();
        assert_eq!(e.code, StepCode::KafkaConsume);
        assert!(ex.lag().await.is_empty());
    }
}
