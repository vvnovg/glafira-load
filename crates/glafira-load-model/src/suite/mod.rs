//! Сьют нагрузки `suites/<имя>/suite.yaml`: схема, выражения, проверка и покрытие профиля
//! (спека L2 §3). Без I/O: приложение, CLI и агент читают сьют одним кодом.

pub mod expr;
pub mod lines;
pub mod schema;

pub use schema::*;

pub const SUITES_DIR: &str = "suites";
pub const SUITE_FILE: &str = "suite.yaml";
pub const FORMAT: u32 = 1;
