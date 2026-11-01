//! Библиотечная часть CLI: команды, консольный вывод, чтение проекта. Бинарник `glafira-load` —
//! тонкая обёртка в `main.rs`; библиотека нужна, чтобы тесты агента (Task 10) поднимали сервер в процессе.

pub mod cmd;
pub mod console;
pub mod exit;
pub mod project;
