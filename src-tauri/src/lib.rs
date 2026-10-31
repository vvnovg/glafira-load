#[macro_use]
pub mod i18n;
pub mod commands;
mod dock_name;
pub mod error;
pub mod git;
pub mod logging;
pub mod policy;
pub mod product;
pub mod project;
pub mod settings;
pub mod state;
pub mod storage;
pub mod sync;

use tauri::Manager;

use crate::state::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            crate::logging::init(&app.path().app_log_dir()?);
            tracing::info!(version = env!("CARGO_PKG_VERSION"), "starting");
            if !crate::dock_name::set(crate::product::PRODUCT_NAME) {
                tracing::warn!("dock name was not set: LaunchServices refused");
            }
            let dir = app.path().app_data_dir()?;
            let policy = crate::policy::sources::load();
            tracing::info!(managed = policy.status.managed, "policy");
            let st = AppState::new(&dir)?.with_policy(policy);
            app.manage(st);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app::get_app_info,
            commands::app::check_update,
            commands::app::install_update,
            commands::settings::get_language_settings,
            commands::settings::set_language,
            commands::settings::get_policy_status,
            commands::project::project_create,
            commands::project::project_open,
            commands::project::project_close,
            commands::project::project_save,
            commands::project::project_recent,
            commands::project::open_path,
        ])
        .run(tauri::generate_context!())
        .expect("error while building tauri application");
}
