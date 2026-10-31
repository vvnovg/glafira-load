//! Имя продукта и идентификаторы — в одном месте. Переименование меняет только этот файл
//! и `tauri.conf.json`; тест ниже следит, чтобы они совпадали.

pub const PRODUCT_NAME: &str = "Glafira for Load Testing";
pub const IDENTIFIER: &str = "io.github.vvnovg.glafira.load";
/// Бинарь настольного приложения (`mainBinaryName`).
pub const MAIN_BINARY: &str = "glafira-load-testing";
/// Бинарь CLI-раннера (этап L2): его имя печатается в подсказках уже сейчас.
pub const RUNNER_BINARY: &str = "glafira-load";
pub const KEYCHAIN_SERVICE: &str = "glafira-load";
pub const POLICY_VENDOR: &str = "Glafira";
pub const POLICY_PRODUCT: &str = "LoadTesting";

#[cfg(test)]
mod tests {
    #[test]
    fn matches_tauri_conf() {
        let conf: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        assert_eq!(conf["identifier"], super::IDENTIFIER);
        assert_eq!(conf["productName"], super::PRODUCT_NAME);
        assert_eq!(conf["mainBinaryName"], super::MAIN_BINARY);
        assert_eq!(conf["app"]["windows"][0]["title"], super::PRODUCT_NAME);
    }
}
