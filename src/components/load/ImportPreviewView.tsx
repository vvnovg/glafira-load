import { useTranslation } from "react-i18next";
import type { TemplatePreview } from "../../api/types";
import { kindFromId } from "../../lib/units";
import { figuresText } from "./figures";

/** Предпросмотр импорта: счётчики, ошибки по строкам, дифф цифр; «Применить» только без ошибок и не в проекте только для чтения. */
export function ImportPreviewView({ preview, onApply, onCancel, busy, readOnly = false }: { preview: TemplatePreview; onApply: () => void; onCancel: () => void; busy: boolean; readOnly?: boolean }) {
  const { t } = useTranslation();
  return (
    <section className="panel import-preview">
      <h3>{t("prodLoad.previewTitle")}</h3>
      <p>{t("prodLoad.previewCounts", { filled: preview.filled.length, empty: preview.empty.length, unknown: preview.unknown_ids.length, invalid: preview.invalid.length })}</p>
      <p className="muted">{t("prodLoad.sourceLine", { stand: preview.source.stand || "—", from: preview.source.period_from || "—", to: preview.source.period_to || "—", by: preview.source.filled_by || "—" })}</p>
      {preview.skipped_rows > 0 && <p className="warn">{t("prodLoad.skippedRows", { count: preview.skipped_rows })}</p>}
      {preview.invalid.length > 0 && (
        <table className="import-warnings">
          <thead>
            <tr>
              <th>{t("prodLoad.row")}</th>
              <th>ID</th>
              <th>{t("prodLoad.column")}</th>
              <th>{t("prodLoad.value")}</th>
              <th>{t("prodLoad.reason")}</th>
            </tr>
          </thead>
          <tbody>
            {preview.invalid.map((c) => (
              <tr key={`${c.row}:${c.column}`}>
                <td>{c.row}</td>
                <td>
                  <code>{c.id}</code>
                </td>
                <td>{c.column}</td>
                <td>{c.value}</td>
                <td>{t(`prodLoad.reasons.${c.reason}`)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      {preview.unknown_ids.length > 0 && (
        <details>
          <summary>{t("prodLoad.unknown", { count: preview.unknown_ids.length })}</summary>
          <ul>
            {preview.unknown_ids.map((id) => (
              <li key={id}>
                <code>{id}</code>
              </li>
            ))}
          </ul>
        </details>
      )}
      {preview.diff.length > 0 && (
        <table>
          <thead>
            <tr>
              <th>ID</th>
              <th>{t("prodLoad.before")}</th>
              <th>{t("prodLoad.after")}</th>
            </tr>
          </thead>
          <tbody>
            {preview.diff.map((d) => (
              <tr key={d.id}>
                <td>
                  <code>{d.id}</code>
                </td>
                <td className="muted">{figuresText(t, kindFromId(d.id), d.old)}</td>
                <td>{figuresText(t, kindFromId(d.id), d.new)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      <div className="row">
        <button type="button" disabled={!preview.result || preview.invalid.length > 0 || busy || readOnly} onClick={onApply}>
          {t("prodLoad.apply")}
        </button>
        <button type="button" onClick={onCancel}>
          {t("common.cancel")}
        </button>
      </div>
    </section>
  );
}
