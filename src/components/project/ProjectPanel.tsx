import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../api/client";
import type { EnvKind, Environment, Manifest, ManifestProblem, Project, ScanProgress, ScanReport, Service } from "../../api/types";
import { projectRelative } from "../../lib/paths";
import { useHelpChapter } from "../../state/helpContext";
import { tabId, useWorkspace } from "../../state/workspace";
import { ErrorBanner } from "../common/ErrorBanner";
import { Hint } from "../common/Hint";

const ENV_KINDS: EnvKind[] = ["dev", "test", "stage", "prod"];
/** Формат ID сервиса и стенда, как `is_id` в Rust. */
const ID_PATTERN = "[a-z0-9\\-]{1,64}";

/** Копия манифеста для черновика (structuredClone нет в WebKit macOS 11–12). */
function cloneManifest(m: Manifest): Manifest {
  return JSON.parse(JSON.stringify(m)) as Manifest;
}

function problemText(t: (k: string, o?: Record<string, unknown>) => string, p: ManifestProblem): string {
  return "id" in p ? t(`project.problems.${p.kind}`, { id: p.id }) : "format" in p ? t("project.problems.bad_format", { format: p.format }) : t("project.problems.empty_name");
}

/** Форма нового проекта: имя и пустая папка. */
function NewProject() {
  const { t } = useTranslation();
  const qc = useQueryClient();
  const { open, close } = useWorkspace();
  const [name, setName] = useState("");
  const [folder, setFolder] = useState<string | null>(null);
  const choose = async () => setFolder((await openDialog({ directory: true, multiple: false, title: t("project.chooseFolder") })) ?? null);
  const create = useMutation({
    mutationFn: () => api.projectCreate(folder ?? "", name.trim()),
    onSuccess: (p) => {
      void qc.invalidateQueries({ queryKey: ["projects"] });
      close(tabId({ kind: "project", path: null }));
      open({ kind: "project", path: p.path });
    },
  });
  return (
    <div className="page">
      <div className="toolbar">
        <h2>{t("project.newTitle")}</h2>
      </div>
      <div className="form stack">
        <div className="field">
          <label htmlFor="project-name">{t("project.name")}</label>
          <input id="project-name" value={name} onChange={(e) => setName(e.target.value)} />
        </div>
        <div className="field">
          <span className="field-label">
            {t("project.folder")} <Hint k="hints.project.folder" label={t("project.folder")} />
          </span>
          <div className="row">
            <code>{folder ?? t("project.noFolder")}</code>
            <button type="button" onClick={choose}>
              {t("project.chooseFolder")}
            </button>
          </div>
        </div>
        {create.error && <ErrorBanner error={create.error} />}
        <button type="button" className="primary" disabled={!folder || !name.trim() || create.isPending} onClick={() => create.mutate()}>
          {t("project.create")}
        </button>
      </div>
    </div>
  );
}

function ServiceRow({ s, project, onChange, onRemove, missing }: { s: Service; project: string; onChange: (s: Service) => void; onRemove: () => void; missing: boolean }) {
  const { t } = useTranslation();
  const choose = async () => {
    const dir = await openDialog({ directory: true, multiple: false, defaultPath: project });
    if (dir) onChange({ ...s, path: projectRelative(project, dir) });
  };
  return (
    <fieldset className="panel">
      <div className="row">
        <label>
          {t("project.serviceId")}
          <input aria-label={t("project.serviceId")} value={s.id} pattern={ID_PATTERN} onChange={(e) => onChange({ ...s, id: e.target.value })} />
        </label>
        <label>
          {t("project.serviceName")}
          <input value={s.name} onChange={(e) => onChange({ ...s, name: e.target.value })} />
        </label>
        <button type="button" className="icon" aria-label={t("common.delete")} onClick={onRemove}>
          ×
        </button>
      </div>
      <div className="row">
        <code className="path-field">{s.path || t("project.noFolder")}</code>
        <button type="button" onClick={choose}>
          {t("project.chooseFolder")}
        </button>
        {missing && <span className="warn">{t("project.serviceMissing")}</span>}
      </div>
    </fieldset>
  );
}

function EnvRow({ e, onChange, onRemove }: { e: Environment; onChange: (e: Environment) => void; onRemove: () => void }) {
  const { t } = useTranslation();
  const field = (key: "http" | "kafka" | "grpc" | "sql") => (
    <label key={key}>
      {t(`project.env.${key}`)}
      <input value={e[key] ?? ""} onChange={(ev) => onChange({ ...e, [key]: ev.target.value || undefined })} />
    </label>
  );
  return (
    <fieldset className="panel">
      <div className="row">
        <label>
          {t("project.envId")}
          <input aria-label={t("project.envId")} value={e.id} pattern={ID_PATTERN} onChange={(ev) => onChange({ ...e, id: ev.target.value })} />
        </label>
        <label>
          {t("project.envKind")}
          <select value={e.kind} onChange={(ev) => onChange({ ...e, kind: ev.target.value as EnvKind })}>
            {ENV_KINDS.map((k) => (
              <option key={k} value={k}>
                {k.toUpperCase()}
              </option>
            ))}
          </select>
        </label>
        <button type="button" className="icon" aria-label={t("common.delete")} onClick={onRemove}>
          ×
        </button>
      </div>
      <div className="row">{(["http", "kafka", "grpc", "sql"] as const).map(field)}</div>
    </fieldset>
  );
}

function ExistingProject({ path }: { path: string }) {
  const { t } = useTranslation();
  const qc = useQueryClient();
  const q = useQuery({ queryKey: ["project", path], queryFn: () => api.projectOpen(path) });
  const [draft, setDraft] = useState<Manifest | null>(null);
  const [progress, setProgress] = useState<ScanProgress | null>(null);
  const [reports, setReports] = useState<ScanReport[]>([]);
  useEffect(() => {
    if (q.data) setDraft(cloneManifest(q.data.manifest));
  }, [q.data]);
  const save = useMutation({
    mutationFn: (m: Manifest) => api.projectSave(path, m),
    onSuccess: (p: Project) => {
      qc.setQueryData(["project", path], p);
      void qc.invalidateQueries({ queryKey: ["projects"] });
    },
  });
  const scanAll = useMutation({
    mutationFn: () => api.serviceScan(path, null, setProgress),
    onSuccess: (r) => {
      setReports(r);
      setProgress(null);
      void qc.invalidateQueries({ queryKey: ["registry", path] });
    },
    onError: () => setProgress(null),
  });
  const cancel = useMutation({ mutationFn: () => api.scanCancel(path, null) });
  const openFolder = useMutation({ mutationFn: () => api.openPath(path, path) });

  if (q.error) return <ErrorBanner error={q.error} onRetry={() => q.refetch()} />;
  const p = q.data;
  if (!p || !draft) return <p className="empty">{t("common.loading")}</p>;
  const update = (patch: Partial<Manifest>) => setDraft({ ...draft, ...patch });
  const dirty = JSON.stringify(draft) !== JSON.stringify(p.manifest);

  return (
    <div className="page">
      <div className="toolbar">
        <h2>{p.manifest.name}</h2>
        <button type="button" onClick={() => openFolder.mutate()}>
          {t("project.openFolder")}
        </button>
        <button type="button" disabled={scanAll.isPending || p.readOnly} onClick={() => scanAll.mutate()}>
          {t("project.scanAll")}
        </button>
        {scanAll.isPending && (
          <button type="button" disabled={cancel.isPending} onClick={() => cancel.mutate()}>
            {t("common.cancel")}
          </button>
        )}
      </div>
      {p.readOnly && (
        <div className="notice" role="alert">
          <strong>{t("project.readOnly")}</strong>
          <ul>
            {p.problems.map((pr, i) => (
              <li key={i}>{problemText(t, pr)}</li>
            ))}
          </ul>
        </div>
      )}
      {progress && (
        <p className="muted">
          {t("project.scanProgress", { service: progress.service, done: progress.done, total: progress.total, file: progress.file })}
        </p>
      )}
      {openFolder.error && <ErrorBanner error={openFolder.error} />}
      {cancel.error && <ErrorBanner error={cancel.error} />}
      {scanAll.error && <ErrorBanner error={scanAll.error} />}
      {reports.length > 0 && (
        <ul className="scan-results">
          {reports.map((r) => (
            <li key={r.service}>
              {t("project.scanResult", { service: r.service, version: r.git.version, found: r.operationsFound, added: r.summary.added.length, changed: r.summary.changed.length, removed: r.summary.removed.length, errors: r.errors.length })}
            </li>
          ))}
        </ul>
      )}
      <fieldset className="form stack" disabled={p.readOnly}>
        <div className="field">
          <label htmlFor="manifest-name">{t("project.name")}</label>
          <input id="manifest-name" value={draft.name} onChange={(e) => update({ name: e.target.value })} />
          <code className="muted">{path}</code>
        </div>
        <h3>
          {t("project.services")} <Hint k="hints.project.services" label={t("project.services")} />
        </h3>
        {draft.services.map((s, i) => (
          <ServiceRow
            key={i}
            s={s}
            project={path}
            missing={p.missingServices.includes(s.id)}
            onChange={(next) => update({ services: draft.services.map((x, j) => (j === i ? next : x)) })}
            onRemove={() => update({ services: draft.services.filter((_, j) => j !== i) })}
          />
        ))}
        <button type="button" onClick={() => update({ services: [...draft.services, { id: "", name: "", path: "", contracts: [] }] })}>
          {t("project.addService")}
        </button>
        <h3>
          {t("project.environments")} <Hint k="hints.project.environments" label={t("project.environments")} />
        </h3>
        {draft.environments.map((e, i) => (
          <EnvRow
            key={i}
            e={e}
            onChange={(next) => update({ environments: draft.environments.map((x, j) => (j === i ? next : x)) })}
            onRemove={() => update({ environments: draft.environments.filter((_, j) => j !== i) })}
          />
        ))}
        <button type="button" onClick={() => update({ environments: [...draft.environments, { id: "", kind: "dev" }] })}>
          {t("project.addEnvironment")}
        </button>
        {save.error && <ErrorBanner error={save.error} />}
        <div className="row">
          <button type="button" className="primary" disabled={!dirty || save.isPending} onClick={() => save.mutate(draft)}>
            {t("common.save")}
          </button>
          <button type="button" disabled={!dirty} onClick={() => setDraft(cloneManifest(p.manifest))}>
            {t("project.revert")}
          </button>
        </div>
      </fieldset>
    </div>
  );
}

export function ProjectPanel({ path }: { path: string | null }) {
  useHelpChapter(tabId({ kind: "project", path }), "project");
  return path ? <ExistingProject path={path} /> : <NewProject />;
}
