import type { AppInfo } from "../../api/types";
import { RELEASES_DOWNLOAD_URL } from "../../lib/links";

/**
 * Команда установки агента для платформы, на которой открыто «О программе»: имя артефакта релиза —
 * `glafira-load-<os>-<arch>[.exe]` со значениями `std::env::consts::{OS, ARCH}`, как их отдаёт `get_app_info`.
 */
export function agentInstallCommand(info: Pick<AppInfo, "os" | "arch" | "version">): string {
  const base = `${RELEASES_DOWNLOAD_URL}/v${info.version}/glafira-load-${info.os}-${info.arch}`;
  // -f: на 404 (нет такой сборки в релизе) curl завершается с ошибкой, а не сохраняет страницу ошибки как бинарник.
  if (info.os === "windows") return `curl.exe -fL ${base}.exe --create-dirs -o C:\\glafira-load\\glafira-load.exe`;
  return `sudo curl -fL ${base} -o /usr/local/bin/glafira-load && sudo chmod +x /usr/local/bin/glafira-load`;
}
