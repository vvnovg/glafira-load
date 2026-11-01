import { useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../api/client";
import type { SmokeReport } from "../../api/types";
import { ErrorBanner } from "../common/ErrorBanner";
import { SummaryTable } from "./SummaryTable";

/** Начало прогона: `started_at` (RFC 3339, UTC) до минут — так же названа папка прогона. */
const when = (iso: string): string => iso.slice(0, 16).replace("T", " ");

/** `smoke.yaml` таблицей: сценарий → шаг → ок или код и причина. */
export function SmokeReportTable({ report }: { report: SmokeReport }) {
  const { t } = useTranslation();
  return (
    <section>
      <h4>{t("run.smoke.title")}</h4>
      <table>
        <thead>
          <tr>
            <th>{t("run.smoke.scenario")}</th>
            <th>{t("run.smoke.step")}</th>
            <th>{t("run.smoke.operation")}</th>
            <th>{t("run.smoke.result")}</th>
          </tr>
        </thead>
        <tbody>
          {report.scenarios.flatMap((s) =>
            s.steps.map((step) => (
              <tr key={`${s.id}/${step.id}`}>
                <td>{s.id}</td>
                <td>{step.id}</td>
                <td>
                  <code>{step.operation}</code>
                </td>
                <td className={step.ok ? undefined : "warn"}>{step.ok ? t("run.smoke.ok") : [step.code, step.message].filter(Boolean).join(": ")}</td>
              </tr>
            )),
          )}
        </tbody>
      </table>
    </section>
  );
}

/** Папки `runs/` с датой, сьютом, профилем и статусом; выбор показывает `summary.yaml` или `smoke.yaml`. */
export function PastRuns({ path }: { path: string }) {
  const { t } = useTranslation();
  const runs = useQuery({ queryKey: ["runs", path], queryFn: () => api.runList(path) });
  const [dir, setDir] = useState<string | null>(null);
  const details = useQuery({ queryKey: ["run", path, dir], queryFn: () => api.runGet(path, dir!), enabled: !!dir });
  return (
    <section>
      <h3>{t("run.past.title")}</h3>
      {runs.error && <ErrorBanner error={runs.error} onRetry={() => void runs.refetch()} />}
      {runs.isPending && <p className="muted">{t("common.loading")}</p>}
      {runs.data?.length === 0 && <p className="muted">{t("run.past.empty")}</p>}
      {runs.data && runs.data.length > 0 && (
        <table>
          <thead>
            <tr>
              <th>{t("run.past.started")}</th>
              <th>{t("run.past.suite")}</th>
              <th>{t("run.past.profile")}</th>
              <th>{t("run.past.env")}</th>
              <th>{t("run.past.mode")}</th>
              <th>{t("run.past.status")}</th>
            </tr>
          </thead>
          <tbody>
            {runs.data.map((m) => (
              // Папка прогона приходит из `run_list` (`dir`), фронтенд её не вычисляет.
              <tr key={m.dir} className={m.dir === dir ? "selected" : undefined}>
                <td>
                  <button type="button" className="link" aria-current={m.dir === dir || undefined} onClick={() => setDir(m.dir)}>
                    {when(m.started_at)}
                  </button>
                </td>
                <td>{m.suite}</td>
                <td>{m.profile}</td>
                <td>{m.env}</td>
                <td>{t(`run.modes.${m.mode}`)}</td>
                <td>{m.status ? t(`run.runStatus.${m.status}`) : <span className="warn">{t("run.past.inProgress")}</span>}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      {dir && details.isFetching && !details.data && <p className="muted">{t("common.loading")}</p>}
      {dir && details.error && <ErrorBanner error={details.error} onRetry={() => void details.refetch()} />}
      {dir && details.data?.summary && <SummaryTable summary={details.data.summary} path={path} dir={details.data.dir ?? dir} />}
      {dir && details.data?.smoke && <SmokeReportTable report={details.data.smoke} />}
      {dir && details.data && !details.data.summary && !details.data.smoke && <p className="muted">{t("run.past.noDetails")}</p>}
    </section>
  );
}
