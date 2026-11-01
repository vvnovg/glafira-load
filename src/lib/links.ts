import { openUrl } from "@tauri-apps/plugin-opener";

/** Страница поддержки проекта — один адрес на все входы: низ левой панели, «О программе», меню Help. */
export const SUPPORT_URL = "https://github.com/vvnovg/glafira-load-releases/blob/main/SUPPORT.md";
/** Текст лицензии Apache 2.0 в репозитории исходного кода. */
export const LICENSE_URL = "https://github.com/vvnovg/glafira-load/blob/main/LICENSE";
/** Корень ссылок на файлы релизов: `<корень>/v<версия>/<файл>`. */
export const RELEASES_DOWNLOAD_URL = "https://github.com/vvnovg/glafira-load-releases/releases/download";

/** Открывает страницу поддержки в системном браузере. */
export function openSupport(): void {
  void openUrl(SUPPORT_URL);
}

/** Открывает текст лицензии в системном браузере. */
export function openLicense(): void {
  void openUrl(LICENSE_URL);
}
