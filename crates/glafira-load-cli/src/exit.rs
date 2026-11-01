//! Коды возврата CLI (спека §5.4): 0 завершено, 2 остановлено, 3 ошибка запуска, 4 найдены ошибки.

use std::process::{ExitCode, Termination};

pub const OK: u8 = 0;
pub const STOPPED: u8 = 2;
pub const STARTUP: u8 = 3;
pub const FINDINGS: u8 = 4;

/// Возвращается из `main`: процесс завершается кодом внутри.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Exit(pub u8);

impl Termination for Exit {
    fn report(self) -> ExitCode {
        ExitCode::from(self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_distinct_and_match_the_spec() {
        assert_eq!([OK, STOPPED, STARTUP, FINDINGS], [0, 2, 3, 4]);
        assert_eq!(Exit(FINDINGS).0, 4);
    }
}
