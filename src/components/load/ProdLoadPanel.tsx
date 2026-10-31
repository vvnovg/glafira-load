import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../api/client";
import type { TemplatePreview } from "../../api/types";
import { useHelpChapter } from "../../state/helpContext";
import { tabId } from "../../state/workspace";
import { ErrorBanner } from "../common/ErrorBanner";
import { Hint } from "../common/Hint";
import { ImportPreviewView } from "./ImportPreviewView";
import { templateLabels } from "./templateLabels";

const XLSX = [{ name: "Excel", extensions: ["xlsx"] }];

export function ProdLoadPanel({ path }: { path: string }) {
  const { t } = useTranslation();
  const qc = useQueryClient();
  useHelpChapter(tabId({ kind: "prod-load", path }), "prod-load");
  const project = useQuery({ queryKey: ["project", path], queryFn: () => api.projectView(path) });
  const prod = useQuery({ queryKey: ["prodLoad", path], queryFn: () => api.prodLoadGet(path) });
  const rows = useQuery({ queryKey: ["registry", path, { status: "active" }], queryFn: () => api.registryList(path, { status: "active" }) });
  const [prefill, setPrefill] = useState(true);
  const [exported, setExported] = useState<string | null>(null);
  const [preview, setPreview] = useState<{ file: string; data: TemplatePreview } | null>(null);
  const exportTemplate = useMutation({
    mutationFn: async () => {
      const name = project.data?.manifest.name ?? "project";
      const date = new Date().toISOString().slice(0, 10);
      const file = await saveDialog({ defaultPath: `prod-load-${name}-${date}.xlsx`, filters: XLSX });
      if (!file) return null;
      const count = await api.prodLoadExportTemplate(path, file, templateLabels(t), prefill);
      return { file, count };
    },
    onSuccess: (r) => r && setExported(t("prodLoad.exported", { count: r.count, file: r.file })),
  });
  const importTemplate = useMutation({
    mutationFn: async () => {
      const file = await openDialog({ multiple: false, filters: XLSX });
      if (!file) return null;
      return { file, data: await api.prodLoadPreviewImport(path, file) };
    },
    onSuccess: (p) => setPreview(p),
  });
  const apply = useMutation({
    mutationFn: (file: string) => api.prodLoadApplyImport(path, file),
    onSuccess: (p) => {
      qc.setQueryData(["prodLoad", path], p);
      void qc.invalidateQueries({ queryKey: ["profileComputed", path] });
      setPreview(null);
    },
  });

  if (prod.error) return <ErrorBanner error={prod.error} onRetry={() => prod.refetch()} />;
  const data = prod.data;
  const list = rows.data ?? [];
  if (!data) return <p className="empty">{t("common.loading")}</p>;
  const uncovered = list.filter((r) => !data.operations[r.id]);
  // Пока манифест не прочитан или в нём есть проблемы, цифры не записываются; выгрузка шаблона разрешена.
  const readOnly = !project.data || project.data.readOnly;

  return (
    <div className="page">
      <div className="toolbar">
        <h2>
          {t("prodLoad.title")} <Hint k="hints.prodLoad.title" label={t("prodLoad.title")} />
        </h2>
        <button type="button" disabled={exportTemplate.isPending || list.length === 0} onClick={() => exportTemplate.mutate()}>
          {t("prodLoad.export")}
        </button>
        <label>
          <input type="checkbox" checked={prefill} onChange={(e) => setPrefill(e.target.checked)} /> {t("prodLoad.prefill")}
        </label>
        <button type="button" disabled={importTemplate.isPending || readOnly} onClick={() => importTemplate.mutate()}>
          {t("prodLoad.import")}
        </button>
        {project.data?.readOnly && <span className="muted">{t("common.readOnly")}</span>}
      </div>
      {project.error && <ErrorBanner error={project.error} onRetry={() => project.refetch()} />}
      {exported && <p className="notice">{exported}</p>}
      {exportTemplate.error && <ErrorBanner error={exportTemplate.error} />}
      {importTemplate.error && <ErrorBanner error={importTemplate.error} />}
      {apply.error && <ErrorBanner error={apply.error} />}
      {preview && <ImportPreviewView preview={preview.data} busy={apply.isPending} readOnly={readOnly} onApply={() => apply.mutate(preview.file)} onCancel={() => setPreview(null)} />}
      {data.source.imported_at && <p className="muted">{t("prodLoad.importedAt", { at: data.source.imported_at.slice(0, 16), stand: data.source.stand || "—", file: data.source.file })}</p>}
      {rows.error && <ErrorBanner error={rows.error} onRetry={() => rows.refetch()} />}
      <table>
        <thead>
          <tr>
            <th>ID</th>
            <th>{t("template.columns.kind")}</th>
            <th>{t("template.columns.avg")}</th>
            <th>{t("template.columns.peak")}</th>
            <th>{t("template.columns.errorPct")}</th>
            <th>{t("template.columns.p95")}</th>
            <th>{t("template.columns.comment")}</th>
          </tr>
        </thead>
        <tbody>
          {list.map((r) => {
            const f = data.operations[r.id];
            return (
              <tr key={r.id} className={f ? "" : "muted"}>
                <td>
                  <code>{r.id}</code>
                </td>
                <td>{t(`registry.kinds.${r.kind}`)}</td>
                <td>{f?.avg ?? "—"}</td>
                <td>{f?.peak ?? "—"}</td>
                <td>{f?.error_pct ?? "—"}</td>
                <td>{f ? (r.kind === "kafka_consume" ? (f.lag_max_s !== undefined ? `${f.lag_max_s} s` : "—") : (f.p95_ms ?? "—")) : "—"}</td>
                <td>{f?.comment ?? ""}</td>
              </tr>
            );
          })}
        </tbody>
      </table>
      {uncovered.length > 0 && <p className="warn">{t("prodLoad.uncovered", { count: uncovered.length })}</p>}
    </div>
  );
}
