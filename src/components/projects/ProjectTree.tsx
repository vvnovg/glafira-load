import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../api/client";
import type { RecentProject } from "../../api/types";
import { baseName } from "../../lib/paths";
import { type TabSpec, useWorkspace } from "../../state/workspace";
import { ErrorBanner } from "../common/ErrorBanner";

const COLLAPSED_KEY = "glafira-load.tree.collapsed";

function readCollapsed(): Set<string> {
  try {
    const raw = localStorage.getItem(COLLAPSED_KEY);
    const parsed: unknown = raw ? JSON.parse(raw) : [];
    return new Set(Array.isArray(parsed) ? parsed.filter((x): x is string => typeof x === "string") : []);
  } catch {
    return new Set();
  }
}

function writeCollapsed(ids: Set<string>) {
  try {
    localStorage.setItem(COLLAPSED_KEY, JSON.stringify([...ids]));
  } catch {
    // Хранилище недоступно — состояние живёт только до перезапуска.
  }
}

/** Узел проекта: имя, разделы, сервисы и сьюты (берутся при раскрытии из манифеста и `suite_list`). */
function ProjectNode({ project, collapsed, onToggle }: { project: RecentProject; collapsed: boolean; onToggle: () => void }) {
  const { t } = useTranslation();
  const qc = useQueryClient();
  const open = useWorkspace((s) => s.open);
  const closeForProject = useWorkspace((s) => s.closeForProject);
  const expanded = !collapsed && !project.missing;
  const manifest = useQuery({ queryKey: ["project", project.path], queryFn: () => api.projectView(project.path), enabled: expanded });
  const suites = useQuery({ queryKey: ["suites", project.path], queryFn: () => api.suiteList(project.path), enabled: expanded });
  const remove = useMutation({
    mutationFn: () => api.projectClose(project.path),
    onSuccess: () => {
      closeForProject(project.path);
      void qc.invalidateQueries({ queryKey: ["projects"] });
    },
  });
  const label = project.name ?? baseName(project.path);
  const section = (spec: TabSpec, text: string) => (
    <li role="treeitem" key={text}>
      <button type="button" className="link tree-name" onClick={() => open(spec)}>
        {text}
      </button>
    </li>
  );
  return (
    // Пропавший проект не раскрывается: aria-expanded только у раскрываемого узла, сам узел серый, а не красный.
    <li role="treeitem" aria-expanded={project.missing ? undefined : !collapsed} aria-label={label} className={project.missing ? "tree-folder muted" : "tree-folder"}>
      <div className="tree-conn">
        <button type="button" className="tree-caret" aria-label={t("sidebar.folderToggle", { name: label })} onClick={onToggle} disabled={project.missing}>
          {collapsed ? "▸" : "▾"}
        </button>
        <button type="button" className="link tree-name" title={project.path} disabled={project.missing} onClick={() => open({ kind: "project", path: project.path })}>
          {label}
        </button>
        {project.missing && <span className="muted">{t("sidebar.missing")}</span>}
        <button type="button" className="icon" aria-label={t("sidebar.remove")} onClick={() => remove.mutate()}>
          ×
        </button>
      </div>
      {expanded && (
        <ul role="group" className="tree">
          {section({ kind: "project", path: project.path }, t("sidebar.project"))}
          <li role="treeitem" aria-label={t("sidebar.services")}>
            <span className="muted">{t("sidebar.services")}</span>
            <ul role="group" className="tree">
              {(manifest.data?.manifest.services ?? []).map((s) => (
                <li role="treeitem" key={s.id}>
                  <button type="button" className="link tree-name" onClick={() => open({ kind: "service", path: project.path, serviceId: s.id })}>
                    {s.name || s.id}
                  </button>
                </li>
              ))}
            </ul>
          </li>
          {section({ kind: "registry", path: project.path }, t("sidebar.registry"))}
          {section({ kind: "prod-load", path: project.path }, t("sidebar.prodLoad"))}
          {section({ kind: "profile", path: project.path }, t("sidebar.profile"))}
          <li role="treeitem" aria-label={t("sidebar.suites")}>
            <div className="tree-conn">
              <span className="muted">{t("sidebar.suites")}</span>
            </div>
            <ul role="group" className="tree">
              {(suites.data ?? []).map((s) => (
                <li role="treeitem" key={s.name}>
                  <button type="button" className="link tree-name" title={s.file} onClick={() => open({ kind: "suite", path: project.path, name: s.name })}>
                    {s.name}
                  </button>
                </li>
              ))}
              {suites.data?.length === 0 && <li className="tree-notice">{t("sidebar.noSuites")}</li>}
            </ul>
          </li>
          {section({ kind: "run", path: project.path }, t("sidebar.run"))}
        </ul>
      )}
      {manifest.error && <ErrorBanner error={manifest.error} />}
      {suites.error && <ErrorBanner error={suites.error} onRetry={() => suites.refetch()} />}
      {remove.error && <ErrorBanner error={remove.error} />}
    </li>
  );
}

export function ProjectTree() {
  const { t } = useTranslation();
  const qc = useQueryClient();
  const open = useWorkspace((s) => s.open);
  const [collapsed, setCollapsed] = useState(readCollapsed);
  const projects = useQuery({ queryKey: ["projects"], queryFn: api.projectRecent });
  const openProject = useMutation({
    mutationFn: async () => {
      const path = await openDialog({ directory: true, multiple: false, title: t("sidebar.openProject") });
      if (!path) return null;
      return api.projectOpen(path);
    },
    onSuccess: (p) => {
      if (!p) return;
      qc.setQueryData(["project", p.path], p);
      void qc.invalidateQueries({ queryKey: ["projects"] });
      open({ kind: "project", path: p.path });
    },
  });
  const toggle = (path: string) => {
    const next = new Set(collapsed);
    if (!next.delete(path)) next.add(path);
    writeCollapsed(next);
    setCollapsed(next);
  };

  return (
    <div className="tree" data-testid="project-tree">
      <div className="sidebar-header">
        <strong>{t("sidebar.projects")}</strong>
        <div className="sidebar-actions">
          <button type="button" className="link" onClick={() => openProject.mutate()}>
            {t("sidebar.openProject")}
          </button>
          <button type="button" className="link" onClick={() => open({ kind: "project", path: null })}>
            {t("sidebar.newProject")}
          </button>
        </div>
      </div>
      {openProject.error && <ErrorBanner error={openProject.error} />}
      {projects.error && <ErrorBanner error={projects.error} onRetry={() => projects.refetch()} />}
      {projects.data?.length === 0 && <p className="empty">{t("sidebar.emptyHint")}</p>}
      <ul role="tree" className="tree">
        {(projects.data ?? []).map((p) => (
          <ProjectNode key={p.path} project={p} collapsed={collapsed.has(p.path)} onToggle={() => toggle(p.path)} />
        ))}
      </ul>
    </div>
  );
}
