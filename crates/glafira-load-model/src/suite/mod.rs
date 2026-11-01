//! Сьют нагрузки `suites/<имя>/suite.yaml`: схема, выражения, проверка и покрытие профиля
//! (спека L2 §3). Без I/O: приложение, CLI и агент читают сьют одним кодом.

pub mod check;
pub mod coverage;
pub mod expr;
pub mod lines;
pub mod schema;

pub use check::{DataInfo, Finding, Level, check};
pub use coverage::{Coverage, CoverageRow, coverage, scenario_rates};
pub use schema::*;

pub const SUITES_DIR: &str = "suites";
pub const SUITE_FILE: &str = "suite.yaml";
pub const FORMAT: u32 = 1;
