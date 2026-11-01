import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useId, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import { api } from "../../api/client";
import type { SuiteMeta } from "../../api/types";
import { trapTab } from "../../lib/focusTrap";
import { useWorkspace } from "../../state/workspace";
import { ErrorBanner } from "../common/ErrorBanner";

/** Имя сьюта — папка `suites/<имя>`, как `[a-z0-9-]{1,64}` в Rust. */
export const SUITE_NAME_RE = /^[a-z0-9-]{1,64}$/;

/** Диалог «Создать сьют»: имя и активные операции реестра галочками; `suite_create` пишет заготовку и открывает вкладку. */
export function CreateSuiteDialog({ path, onClose }: { path: string; onClose: () => void }) {
  const { t } = useTranslation();
  const qc = useQueryClient();
  const open = useWorkspace((s) => s.open);
  const root = useRef<HTMLDivElement>(null);
  const uid = useId();
  const [name, setName] = useState("");
  const [query, setQuery] = useState("");
  const [picked, setPicked] = useState<Set<string>>(() => new Set());
  // Тот же ключ, что у «Нагрузки ПРОМ»: список активных операций берётся из общего кэша.
  const ops = useQuery({ queryKey: ["registry", path, { status: "active" }], queryFn: () => api.registryList(path, { status: "active" }) });
  useEffect(() => root.current?.focus(), []);
  // Поиск фильтрует на клиенте; отмеченные галочки переживают смену фильтра.
  const list = useMemo(() => {
    const q = query.trim().toLowerCase();
    return (ops.data ?? []).filter((r) => !q || r.id.toLowerCase().includes(q) || r.name.toLowerCase().includes(q));
  }, [ops.data, query]);
  const trimmed = name.trim();
  const validName = SUITE_NAME_RE.test(trimmed);
  const badName = trimmed !== "" && !validName;
  const create = useMutation({
    mutationFn: () => api.suiteCreate(path, trimmed, [...picked]),
    onSuccess: (meta: SuiteMeta) => {
      // Узел «Сьюты» в дереве перечитывает список и показывает новый сьют.
      void qc.invalidateQueries({ queryKey: ["suites", path] });
      open({ kind: "suite", path, name: meta.name });
      onClose();
    },
  });
  const toggle = (id: string) =>
    setPicked((prev) => {
      const next = new Set(prev);
      if (!next.delete(id)) next.add(id);
      return next;
    });
  // Портал в `body`: диалог открывают и из дерева (`role="tree"`), и из вкладки — в разметке он не их потомок.
  return createPortal(
    <div className="modal-backdrop" onClick={onClose}>
      <div
        ref={root}
        className="modal"
        role="dialog"
        aria-modal="true"
        aria-labelledby={`${uid}-title`}
        tabIndex={-1}
        onClick={(e) => e.stopPropagation()}
        onKeyDown={(e) => {
          if (e.key === "Escape") onClose();
          else if (e.key === "Tab") trapTab(root.current, e);
        }}
      >
        <h3 id={`${uid}-title`}>{t("suite.dialog.title")}</h3>
        <div className="field">
          <label htmlFor={`${uid}-name`}>{t("suite.dialog.name")}</label>
          <input
            id={`${uid}-name`}
            value={name}
            aria-invalid={badName || undefined}
            aria-describedby={badName ? `${uid}-bad` : undefined}
            onChange={(e) => setName(e.target.value)}
          />
          {badName && (
            <span id={`${uid}-bad`} className="warn">
              {t("suite.dialog.badName")}
            </span>
          )}
        </div>
        <input type="search" aria-label={t("suite.dialog.search")} placeholder={t("suite.dialog.search")} value={query} onChange={(e) => setQuery(e.target.value)} />
        {ops.error && <ErrorBanner error={ops.error} onRetry={() => ops.refetch()} />}
        {ops.data && ops.data.length === 0 && <p className="muted">{t("suite.dialog.noOperations")}</p>}
        <ul className="pick-list" aria-label={t("suite.dialog.operations")}>
          {list.map((r) => (
            <li key={r.id}>
              <label>
                <input type="checkbox" checked={picked.has(r.id)} onChange={() => toggle(r.id)} /> <span className={`kind-badge kind-${r.kind}`}>{t(`registry.kinds.${r.kind}`)}</span>{" "}
                <code>{r.id}</code> <span className="muted">{r.name}</span>
              </label>
            </li>
          ))}
        </ul>
        <span className="muted" aria-live="polite">
          {t("suite.dialog.selected", { n: picked.size })}
        </span>
        {create.error && <ErrorBanner error={create.error} />}
        <div className="row">
          <button type="button" className="primary" disabled={!validName || picked.size === 0 || create.isPending} onClick={() => create.mutate()}>
            {t("suite.dialog.create")}
          </button>
          <button type="button" onClick={onClose}>
            {t("common.cancel")}
          </button>
        </div>
      </div>
    </div>,
    document.body,
  );
}
