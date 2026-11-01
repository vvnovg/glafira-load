import { useTranslation } from "react-i18next";

/**
 * Вкладка «Прогон». Пока заготовка: форма запуска, живые графики, итог и прошлые прогоны
 * добавляет задача вкладки «Прогон» (Task 16), заменяя этот файл.
 */
export function RunPanel({ path }: { path: string }) {
  const { t } = useTranslation();
  return (
    <div className="page">
      <div className="toolbar">
        <h2>{t("run.title")}</h2>
      </div>
      <code className="muted">{path}</code>
    </div>
  );
}
