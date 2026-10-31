import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ask } from "@tauri-apps/plugin-dialog";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../api/client";
import type { Address, Manual, Operation } from "../../api/types";
import { joinProjectPath } from "../../lib/paths";
import { ErrorBanner } from "../common/ErrorBanner";
import { SchemaTree } from "./SchemaTree";

export function addressLabel(a: Address): string {
  switch (a.type) {
    case "http":
      return `${a.method} ${a.path}`;
    case "kafka":
      return a.topic;
    case "grpc":
      return `${a.service}/${a.method}`;
    case "sql":
      return a.name;
  }
}

/** Правая панель реестра: адрес, источник, версии, схемы и ручные поля. */
export function OperationDetails({ path, id, clones, readOnly = false, onDeleted }: { path: string; id: string; clones: Record<string, string>; readOnly?: boolean; onDeleted: () => void }) {
  const { t } = useTranslation();
  const qc = useQueryClient();
  const op = useQuery({ queryKey: ["operation", path, id], queryFn: () => api.registryGet(path, id) });
  const [manual, setManual] = useState<Manual>({});
  // Теги правятся строкой: запятая в конце не должна пропадать, пока человек печатает.
  const [tagsText, setTagsText] = useState("");
  useEffect(() => {
    if (op.data) setTagsText((op.data.manual?.tags ?? []).join(", "));
    if (op.data) setManual({ tags: op.data.manual?.tags ?? [], notes: op.data.manual?.notes ?? "", owner: op.data.manual?.owner ?? "" });
  }, [op.data]);
  const invalidate = () => {
    void qc.invalidateQueries({ queryKey: ["registry", path] });
    void qc.invalidateQueries({ queryKey: ["operation", path, id] });
  };
  const save = useMutation({ mutationFn: (m: Manual) => api.registryUpdateManual(path, id, m), onSuccess: invalidate });
  const openSource = useMutation({ mutationFn: (file: string) => api.openPath(path, file) });
  const remove = useMutation({
    mutationFn: async () => {
      if (!(await ask(t("registry.deleteConfirm", { id }), { kind: "warning" }))) return false;
      await api.registryDelete(path, id);
      return true;
    },
    onSuccess: (deleted) => {
      if (deleted) {
        invalidate();
        onDeleted();
      }
    },
  });

  if (op.error) return <ErrorBanner error={op.error} onRetry={() => op.refetch()} />;
  const o: Operation | undefined = op.data;
  if (!o) return <p className="empty">{t("common.loading")}</p>;
  const clone = clones[o.service];
  const sourcePath = o.source.kind === "manual" || !clone ? null : joinProjectPath(clone, o.source.path);

  return (
    <div className="details-pane">
      <h3>
        <code>{o.id}</code>
      </h3>
      <p>
        <span className={o.status === "removed" ? "muted" : ""}>{t(`registry.status.${o.status}`)}</span> · {t(`registry.kinds.${o.kind}`)} · <code>{addressLabel(o.address)}</code>
        {o.streaming && <span className="muted"> · {t("registry.streaming", { mode: o.streaming })}</span>}
        {o.topic_guessed && <span className="warn"> · {t("registry.topicGuessed")}</span>}
      </p>
      <p>
        <strong>{o.name}</strong> {o.summary && <span className="muted">{o.summary}</span>}
      </p>
      <p>
        {t("registry.source", { kind: o.source.kind })}{" "}
        {sourcePath && (
          <>
            <code>{o.source.line ? `${o.source.path}:${o.source.line}` : o.source.path}</code>{" "}
            <button type="button" className="link" onClick={() => openSource.mutate(sourcePath)}>
              {t("service.openFile")}
            </button>
          </>
        )}
      </p>
      <p className="muted">{t("registry.seen", { first: o.first_seen.version, last: o.last_seen.version, at: o.last_seen.at.slice(0, 10) })}</p>
      {o.request && (
        <section>
          <h4>{t("registry.request")}</h4>
          <SchemaTree schema={o.request} />
        </section>
      )}
      {o.response && (
        <section>
          <h4>{t("registry.response")}</h4>
          <SchemaTree schema={o.response} />
        </section>
      )}
      {o.responses &&
        Object.entries(o.responses).map(([code, s]) => (
          <section key={code}>
            <h4>{t("registry.responseCode", { code })}</h4>
            <SchemaTree schema={s} />
          </section>
        ))}
      <section className="form">
        <h4>{t("registry.manual")}</h4>
        <label>
          {t("registry.tags")}
          <input disabled={readOnly} aria-label={t("registry.tags")} value={tagsText} onChange={(e) => {
            setTagsText(e.target.value);
            setManual({ ...manual, tags: e.target.value.split(",").map((s) => s.trim()).filter(Boolean) });
          }} />
        </label>
        <label>
          {t("registry.owner")}
          <input disabled={readOnly} aria-label={t("registry.owner")} value={manual.owner ?? ""} onChange={(e) => setManual({ ...manual, owner: e.target.value })} />
        </label>
        <label>
          {t("registry.notes")}
          <textarea disabled={readOnly} aria-label={t("registry.notes")} value={manual.notes ?? ""} onChange={(e) => setManual({ ...manual, notes: e.target.value })} />
        </label>
        {openSource.error && <ErrorBanner error={openSource.error} />}
        {save.error && <ErrorBanner error={save.error} />}
        {remove.error && <ErrorBanner error={remove.error} />}
        <div className="row">
          <button type="button" disabled={save.isPending || readOnly} onClick={() => save.mutate(manual)}>
            {t("common.save")}
          </button>
          <button type="button" className="danger" disabled={remove.isPending || readOnly} onClick={() => remove.mutate()}>
            {o.source.kind === "manual" ? t("common.delete") : t("registry.deleteRemoved")}
          </button>
        </div>
      </section>
    </div>
  );
}
