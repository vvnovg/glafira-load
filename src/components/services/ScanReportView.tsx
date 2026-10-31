import { useMutation } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { api } from "../../api/client";
import type { Diagnostic, ScanReport } from "../../api/types";
import { joinProjectPath } from "../../lib/paths";
import { useWorkspace } from "../../state/workspace";
import { tr } from "../../i18n/text";
import { ErrorBanner } from "../common/ErrorBanner";

/** Диагностики, сгруппированные по файлу (в порядке первого появления). */
function groupByFile(items: Diagnostic[]): [string, Diagnostic[]][] {
  const groups = new Map<string, Diagnostic[]>();
  for (const d of items) groups.set(d.path, [...(groups.get(d.path) ?? []), d]);
  return [...groups];
}

function DiagnosticList({ items, project, clone, title, open }: { items: Diagnostic[]; project: string; clone: string; title: string; open: boolean }) {
  const { t } = useTranslation();
  const openFile = useMutation({ mutationFn: (file: string) => api.openPath(project, joinProjectPath(clone, file)) });
  if (items.length === 0) return null;
  return (
    <details open={open}>
      <summary>
        {title} ({items.length})
      </summary>
      <ul>
        {groupByFile(items).map(([file, list]) => (
          <li key={file}>
            <button type="button" className="link" onClick={() => openFile.mutate(file)}>
              {t("service.openFile")}
            </button>
            <ul>
              {list.map((d, i) => (
                <li key={i}>
                  <code>{d.line ? `${d.path}:${d.line}` : d.path}</code> {d.message}
                </li>
              ))}
            </ul>
          </li>
        ))}
      </ul>
      {openFile.error && <ErrorBanner error={openFile.error} />}
    </details>
  );
}

/** Итог разбора сервиса: версия, счётчики, предупреждения и ошибки по файлу и строке. */
export function ScanReportView({ report, project, clone }: { report: ScanReport; project: string; clone: string }) {
  const { t } = useTranslation();
  const openTab = useWorkspace((s) => s.open);
  const openScan = useMutation({ mutationFn: () => api.openPath(project, joinProjectPath(project, report.scanFile)) });
  const s = report.summary;
  return (
    <section className="panel">
      {report.git.available ? (
        <p>
          {t("service.gitVersion", { version: report.git.version })} <code className="muted">{report.git.commit.slice(0, 12)}</code>
        </p>
      ) : (
        <p className="warn">{t("service.gitUnavailable", { error: report.git.error ?? "" })}</p>
      )}
      {report.cancelled && <p className="warn">{t("service.cancelled")}</p>}
      {report.notice && <p className="warn">{tr(report.notice)}</p>}
      <p>{t("service.counts", { contracts: report.contracts, found: report.operationsFound, tables: report.tablesFound })}</p>
      {!report.cancelled && !report.notice && <p>{t("service.summary", { added: s.added.length, changed: s.changed.length, removed: s.removed.length, restored: s.restored.length, unchanged: s.unchanged })}</p>}
      <DiagnosticList items={report.errors} project={project} clone={clone} title={t("service.errors")} open />
      <DiagnosticList items={report.warnings} project={project} clone={clone} title={t("service.warnings")} open={false} />
      {!report.cancelled && report.scanFile && (
        <div className="row">
          <button type="button" className="link" onClick={() => openScan.mutate()}>
            {t("service.openScan")}
          </button>
          <button type="button" className="link" onClick={() => openTab({ kind: "registry", path: project })}>
            {t("service.openRegistry")}
          </button>
        </div>
      )}
      {openScan.error && <ErrorBanner error={openScan.error} />}
    </section>
  );
}
