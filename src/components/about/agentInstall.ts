import type { AppInfo } from "../../api/types";
import { RELEASES_DOWNLOAD_URL } from "../../lib/links";

/**
 * Команда установки агента для платформы, на которой открыто «О программе»: имя артефакта релиза —
 * `glafira-load-<os>-<arch>[.exe]` со значениями `std::env::consts::{OS, ARCH}`, как их отдаёт `get_app_info`.
 */
export function agentInstallCommand(info: Pick<AppInfo, "os" | "arch" | "version">): string {
  const base = `${RELEASES_DOWNLOAD_URL}/v${info.version}/glafira-load-${info.os}-${info.arch}`;
  if (info.os === "windows") return `curl.exe -L ${base}.exe -o glafira-load.exe`;
  return `curl -L ${base} -o /usr/local/bin/glafira-load && chmod +x /usr/local/bin/glafira-load`;
}
