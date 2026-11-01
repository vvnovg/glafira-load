#[macro_use]
pub mod i18n;
pub mod agents;
pub mod commands;
mod dock_name;
pub mod error;
pub mod git;
pub mod load;
pub mod logging;
pub mod policy;
pub mod product;
pub mod project;
pub mod runs;
pub mod scan;
pub mod settings;
pub mod state;
pub mod storage;
pub mod suites;
pub mod sync;

use tauri::{Emitter, Manager};

use crate::state::AppState;

/// Событие фронтенду: окно закрывают при идущем прогоне (Task 16 показывает диалог и вызывает
/// `run_stop` или `confirm_close`).
pub const RUN_CLOSE_REQUESTED: &str = "run-close-requested";

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
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let st = window.state::<AppState>();
                if st.runs.is_busy() {
                    api.prevent_close();
                    if let Err(e) = window.emit(RUN_CLOSE_REQUESTED, ()) {
                        tracing::warn!("close prompt was not delivered: {e}");
                    }
                }
            }
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
            commands::project::project_view,
            commands::project::project_close,
            commands::project::project_save,
            commands::project::project_recent,
            commands::project::open_path,
            commands::scan::service_discover_contracts,
            commands::scan::service_scan,
            commands::scan::scan_cancel,
            commands::registry::registry_list,
            commands::registry::registry_get,
            commands::registry::registry_update_manual,
            commands::registry::registry_add_manual,
            commands::registry::registry_delete,
            commands::registry::registry_scans,
            commands::registry::registry_diff,
            commands::registry::registry_tables,
            commands::load::prod_load_export_template,
            commands::load::prod_load_preview_import,
            commands::load::prod_load_apply_import,
            commands::load::prod_load_get,
            commands::load::profile_get,
            commands::load::profile_save,
            commands::load::profile_compute,
            commands::suite::suite_list,
            commands::suite::suite_get,
            commands::suite::suite_check,
            commands::suite::suite_create,
            commands::agent::agent_set_token,
            commands::agent::agent_probe,
            commands::run::run_start,
            commands::run::run_stop,
            commands::run::run_list,
            commands::run::run_get,
            commands::run::confirm_close,
        ])
        .run(tauri::generate_context!())
        .expect("error while building tauri application");
}
