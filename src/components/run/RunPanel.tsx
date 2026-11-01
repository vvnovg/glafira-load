import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useMemo, useRef } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../api/client";
import { currentLocale } from "../../i18n";
import { formatNumber } from "../../lib/format";
import { isActive, useProjectRun } from "../../state/run";
import { ErrorBanner } from "../common/ErrorBanner";
import { Hint } from "../common/Hint";
import { LiveCharts } from "./LiveCharts";
import { OpsTable } from "./OpsTable";
import { PastRuns, SmokeReportTable } from "./PastRuns";
import { RunForm } from "./RunForm";
import { SummaryTable } from "./SummaryTable";

/** Минуты с одним знаком после запятой по языку интерфейса («1,0» / «1.0»). */
const minutes = (seconds: number): string => (seconds / 60).toLocaleString(currentLocale(), { minimumFractionDigits: 1, maximumFractionDigits: 1 });

/** Вкладка «Прогон»: форма, живой ход (фаза, счётчики, графики, операции), итог, прошлые прогоны. */
export function RunPanel({ path }: { path: string }) {
  const { t } = useTranslation();
  const qc = useQueryClient();
  const run = useProjectRun(path);
  const active = isActive(run.status);
  // Завершившийся прогон появляется в списке прошлых.
  const wasActive = useRef(active);
  useEffect(() => {
    if (wasActive.current && !active) void qc.invalidateQueries({ queryKey: ["runs", path] });
    wasActive.current = active;
  }, [active, path, qc]);
  const isSmoke = run.params?.mode === "smoke";
  // Смок живых графиков не имеет (решение плана 5): после завершения читаем `smoke.yaml` (`run_get` принимает и абсолютный путь).
  const smoke = useQuery({ queryKey: ["run", path, run.dir], queryFn: () => api.runGet(path, run.dir!), enabled: run.status === "done" && isSmoke && !!run.dir });
  // eslint-disable-next-line react-hooks/exhaustive-deps
  const ops = useMemo(() => Object.keys(run.series.p95ByOp).sort(), [run.series, run.version]);
  const timeline = run.params?.timeline ?? [];
  const totalMinutes = timeline.length > 0 ? timeline[timeline.length - 1].minute : 0;
  const live = run.status === "running" || run.status === "stopping";
  return (
    <div className="page">
      <div className="toolbar">
        <h2>
          {t("run.title")} <Hint k="hints.run.title" label={t("run.title")} />
        </h2>
      </div>
      <RunForm path={path} />
      {(live || run.status === "done") && !isSmoke && (
        <section className="run-live" aria-busy={live || undefined}>
          <div className="row counters">
            {run.phase && <span>{t("run.phase", { index: run.phase.index + 1, pct: formatNumber(run.phase.pct, 1) })}</span>}
            {run.lastTick && <span className="muted">{t("run.elapsed", { done: minutes(run.lastTick.t), total: formatNumber(totalMinutes, 1) })}</span>}
            <span>{t("run.counters.requests", { n: formatNumber(run.requests, 0) })}</span>
            <span className={run.errors > 0 ? "warn" : undefined}>{t("run.counters.errors", { n: formatNumber(run.errors, 0) })}</span>
            <span className={run.dropped > 0 ? "warn" : undefined}>{t("run.counters.dropped", { n: formatNumber(run.dropped, 0) })}</span>
          </div>
          <progress max={totalMinutes * 60 || 1} value={run.lastTick?.t ?? 0} aria-label={t("run.progress")} />
          <LiveCharts series={run.series} version={run.version} ops={ops} />
          <OpsTable tick={run.lastTick} />
        </section>
      )}
      {/* Примеры ошибок приходят и в смоке (событие `error`), и в прогоне. */}
      {run.samples.length > 0 && (live || run.status === "done") && (
        <details>
          <summary>{t("run.samples")}</summary>
          <ul>
            {run.samples.map((s, i) => (
              <li key={i}>
                <code>{s.code}</code> {s.operation} · {s.scenario}/{s.step} · {s.agent}: {s.message}
              </li>
            ))}
          </ul>
        </details>
      )}
      {run.status === "done" && run.result && !isSmoke && <SummaryTable summary={run.result.summary} path={path} dir={run.dir} />}
      {run.status === "done" && isSmoke && smoke.error && <ErrorBanner error={smoke.error} onRetry={() => void smoke.refetch()} />}
      {run.status === "done" && isSmoke && smoke.data?.smoke && <SmokeReportTable report={smoke.data.smoke} />}
      <PastRuns path={path} />
    </div>
  );
}
