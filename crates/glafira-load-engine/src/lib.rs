//! Движок нагрузки Glafira for Load Testing (спека L2 §5): протокол агента, метрики,
//! контекст итерации, проверки, извлечение, исполнители шагов и планировщик.
//! Крейт не зависит от `tauri`; пользовательские строки — английские.

pub mod agent_api;
pub mod agent_link;
pub mod avro;
pub mod context;
pub mod errors;
pub mod executor;
pub mod expect;
pub mod extract;
pub mod http;
pub mod kafka;
pub mod local;
pub mod metrics;
pub mod run_files;
pub mod scheduler;
pub mod summary;
