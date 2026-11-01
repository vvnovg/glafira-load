//! Меню приложения: пункт «Quit» (Cmd+Q) на macOS — наш, а не предопределённый.
//!
//! Без своего меню Tauri ставит на macOS `Menu::default`, где «Quit» — это
//! `PredefinedMenuItem::quit`: muda шлёт селектор `terminate:` прямо в `NSApplication`, а tao не
//! реализует `applicationShouldTerminate:`. Поэтому Cmd+Q даёт только `RunEvent::Exit` — без
//! `ExitRequested`, и выход нельзя задержать ради диалога о прогоне. Наш пункт приходит в
//! `on_menu_event`, и решение принимает приложение (`quit_action`).
//!
//! На Windows и Linux Tauri меню по умолчанию не ставит — там ничего не меняем.

/// Id нашего пункта «Quit».
pub const QUIT_ID: &str = "quit";

/// Сочетание клавиш нашего пункта «Quit».
pub const QUIT_ACCELERATOR: &str = "CmdOrCtrl+Q";

/// Что делать по пункту «Quit».
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuitAction {
    /// Идёт прогон — не выходить, показать диалог закрытия (`RUN_CLOSE_REQUESTED`).
    Hold,
    /// Выйти сразу (`app.exit(0)`).
    Exit,
}

/// Решение по пункту «Quit»: идёт прогон и есть окно для диалога — спросить, иначе выйти.
/// Прогон без окна не держит выход: спросить негде, а `RunEvent::Exit` остановит прогоны сам.
pub fn quit_action(busy: bool, has_window: bool) -> QuitAction {
    if busy && has_window {
        QuitAction::Hold
    } else {
        QuitAction::Exit
    }
}

/// Позиция предопределённого «Quit» в подменю приложения. `labels` — тексты пунктов подменю
/// (`None` — не предопределённый пункт или текст не прочитался). Текст muda хранит с мнемоникой
/// (`&Quit`), на macOS — «Quit <имя приложения>».
pub fn quit_position(labels: &[Option<String>]) -> Option<usize> {
    labels.iter().rposition(|l| {
        l.as_deref()
            .is_some_and(|t| t.trim_start_matches('&').starts_with("Quit"))
    })
}

/// Меню приложения на macOS: `Menu::default`, в котором предопределённый «Quit» заменён нашим
/// пунктом `QUIT_ID` с тем же текстом и `QUIT_ACCELERATOR`. Предопределённого «Quit» не нашлось
/// (Tauri поменял меню по умолчанию) — наш пункт добавляется в конец подменю приложения.
#[cfg(target_os = "macos")]
pub fn app_menu<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
) -> tauri::Result<tauri::menu::Menu<R>> {
    use tauri::menu::{Menu, MenuItem, MenuItemKind};

    let menu = Menu::default(app)?;
    let Some(MenuItemKind::Submenu(app_submenu)) = menu.items()?.into_iter().next() else {
        tracing::warn!("default menu has no app submenu, quit item was not replaced");
        return Ok(menu);
    };
    let labels: Vec<Option<String>> = app_submenu
        .items()?
        .iter()
        .map(|i| match i {
            MenuItemKind::Predefined(p) => p.text().ok(),
            _ => None,
        })
        .collect();
    let position = quit_position(&labels);
    let text = position
        .and_then(|i| labels[i].clone())
        .unwrap_or_else(|| format!("Quit {}", crate::product::PRODUCT_NAME));
    let quit = MenuItem::with_id(app, QUIT_ID, text, true, Some(QUIT_ACCELERATOR))?;
    match position {
        Some(i) => {
            app_submenu.remove_at(i)?;
            app_submenu.insert(&quit, i)?;
        }
        None => {
            tracing::warn!("predefined quit item not found, appending ours");
            app_submenu.append(&quit)?;
        }
    }
    Ok(menu)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quit_holds_only_during_a_run_with_a_window() {
        assert_eq!(quit_action(true, true), QuitAction::Hold, "прогон — диалог");
        assert_eq!(quit_action(false, true), QuitAction::Exit, "прогона нет");
        assert_eq!(
            quit_action(true, false),
            QuitAction::Exit,
            "окна нет — спрашивать негде"
        );
        assert_eq!(quit_action(false, false), QuitAction::Exit);
    }

    #[test]
    fn predefined_quit_is_found_in_the_app_submenu() {
        // Подменю приложения из `Menu::default` (tauri 2.12): About, sep, Services, sep, Hide,
        // Hide Others, sep, Quit. Все они предопределённые; у разделителей текст пустой.
        let sep = || Some(String::new());
        let labels = [
            Some("About Glafira".to_string()),
            sep(),
            Some("Services".to_string()),
            sep(),
            Some("Hide Glafira".to_string()),
            Some("Hide Others".to_string()),
            sep(),
            Some("Quit Glafira".to_string()),
        ];
        assert_eq!(quit_position(&labels), Some(7));
        assert_eq!(
            quit_position(&[Some("&Quit".to_string()), None]),
            Some(0),
            "мнемоника не мешает"
        );
        assert_eq!(quit_position(&[Some("Hide".to_string()), None]), None);
        assert_eq!(quit_position(&[]), None);
    }
}
