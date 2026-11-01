import { useTranslation } from "react-i18next";
import type { Finding } from "../../api/types";

export const hasErrors = (findings: Finding[]): boolean => findings.some((f) => f.level === "error");

/** Таблица находок `check`: уровень, файл:строка (кнопка открывает файл), сообщение. */
export function CheckReport({ findings, onOpen }: { findings: Finding[]; onOpen: (file: string) => void }) {
  const { t } = useTranslation();
  if (findings.length === 0) return <p className="muted">{t("suite.noFindings")}</p>;
  return (
    <table className="findings">
      <thead>
        <tr>
          <th>{t("suite.column.level")}</th>
          <th>{t("suite.column.where")}</th>
          <th>{t("suite.column.message")}</th>
        </tr>
      </thead>
      <tbody>
        {findings.map((f, i) => (
          <tr key={i}>
            <td>
              <span className={`sev sev-${f.level}`}>{t(`suite.level.${f.level}`)}</span>
            </td>
            <td>
              <button type="button" className="link" title={t("suite.openFile")} onClick={() => onOpen(f.file)}>
                <code>{f.line === null ? f.file : `${f.file}:${f.line}`}</code>
              </button>
            </td>
            <td>{f.message}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}
