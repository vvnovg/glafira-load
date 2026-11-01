import { useMutation } from "@tanstack/react-query";
import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../api/client";
import type { AgentSummary, OpSummary } from "../../api/types";
import { formatNumber } from "../../lib/format";
import { joinProjectPath } from "../../lib/paths";
import { ErrorBanner } from "../common/ErrorBanner";
import { type Sort, sortRows, SortTh } from "./sorting";

/** Колонки итога: поле `OpSummary` (camelCase, как в `summary.yaml`), ключ подписи `run.summary.*`, знаков после запятой. */
const COLUMNS: { key: keyof OpSummary; label: string; digits: number }[] = [
  { key: "n", label: "requests", digits: 0 },
  { key: "rpsAvg", label: "rpsAvg", digits: 1 },
  { key: "rpsPeak", label: "rpsPeak", digits: 1 },
  { key: "errPct", label: "errPct", digits: 2 },
  { key: "dropped", label: "dropped", digits: 0 },
  { key: "p50", label: "p50", digits: 1 },
  { key: "p95", label: "p95", digits: 1 },
  { key: "p99", label: "p99", digits: 1 },
  { key: "max", label: "max", digits: 1 },
  { key: "target", label: "target", digits: 1 },
  { key: "achievedPct", label: "achieved", digits: 1 },
];

/**
 * Итог прогона (`summary.yaml`) по операциям с сортировкой по колонкам и «Открыть папку».
 * `dir` — папка прогона: `runs/<папка>` из `run_list` или абсолютный путь из `run_start`.
 */
export function SummaryTable({ summary, path, dir }: { summary: AgentSummary; path: string; dir: string | null }) {
  const { t } = useTranslation();
  const [sort, setSort] = useState<Sort<OpSummary>>({ key: "id", dir: "asc" });
  const rows = useMemo(() => sortRows(summary.ops, sort), [summary, sort]);
  const openFolder = useMutation({ mutationFn: (d: string) => api.openPath(path, joinProjectPath(path, d)) });
  return (
    <section>
      <div className="row">
        <h4>{t("run.summary.title")}</h4>
        <span className="muted">
          {t(`run.runStatus.${summary.status}`)} · {t("run.summary.seconds", { seconds: summary.seconds })}
        </span>
        {dir && (
          <button type="button" disabled={openFolder.isPending} onClick={() => openFolder.mutate(dir)}>
            {t("run.summary.openFolder")}
          </button>
        )}
      </div>
      {openFolder.error && <ErrorBanner error={openFolder.error} />}
      <table>
        <thead>
          <tr>
            <SortTh sort={sort} column="id" label={t("run.ops.operation")} onChange={setSort} />
            {COLUMNS.map((c) => (
              <SortTh key={String(c.key)} sort={sort} column={c.key} label={t(`run.summary.${c.label}`)} onChange={setSort} />
            ))}
          </tr>
        </thead>
        <tbody>
          {rows.map((r) => (
            <tr key={r.id}>
              <td>
                <code>{r.id}</code>
              </td>
              {COLUMNS.map((c) => {
                const v = r[c.key];
                return (
                  <td key={String(c.key)} className={c.key === "errPct" && r.errPct > 0 ? "warn" : undefined}>
                    {typeof v === "number" ? formatNumber(v, c.digits) : "—"}
                  </td>
                );
              })}
            </tr>
          ))}
        </tbody>
      </table>
    </section>
  );
}
