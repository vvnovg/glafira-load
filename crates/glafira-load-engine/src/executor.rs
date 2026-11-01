//! Исполнитель шага: результат и трейт (спека L2 §5.2). Диспетчер `Executors` — Task 6.

use std::collections::BTreeMap;

use async_trait::async_trait;
use glafira_load_model::suite::Step;
use serde_json::Value;

use crate::context::IterationCtx;
use crate::errors::StepError;

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
