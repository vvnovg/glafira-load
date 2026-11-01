import { useTranslation } from "react-i18next";

/**
 * Вкладка «Сьют». Пока заготовка: дерево шагов, отчёт `check`, покрытие профиля
 * и создание из реестра добавляет задача вкладки «Сьют» (Task 15), заменяя этот файл.
 */
export function SuitePanel({ path, name }: { path: string; name: string }) {
  const { t } = useTranslation();
  return (
    <div className="page">
      <div className="toolbar">
        <h2>{t("suite.title", { name })}</h2>
      </div>
      <code className="muted">{path}</code>
    </div>
  );
}
