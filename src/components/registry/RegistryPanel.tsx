import { useQuery } from "@tanstack/react-query";
import { observeElementRect, useVirtualizer } from "@tanstack/react-virtual";
import { type KeyboardEvent, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../api/client";
import type { OpKind, OperationRow, RegistryFilter, ScanMeta, SourceKind, Status } from "../../api/types";
import { joinProjectPath } from "../../lib/paths";
import { useDebouncedValue } from "../../lib/useDebouncedValue";
import { useHelpChapter } from "../../state/helpContext";
import { tabId } from "../../state/workspace";
import { ErrorBanner } from "../common/ErrorBanner";
import { Hint } from "../common/Hint";
import { ManualOperationForm } from "./ManualOperationForm";
import { OperationDetails } from "./OperationDetails";

const KINDS: OpKind[] = ["http", "kafka_produce", "kafka_consume", "grpc", "sql"];
const SOURCES: SourceKind[] = ["openapi", "proto", "asyncapi", "schema", "sql", "spring", "manual"];
const ROW_HEIGHT = 32;
const PAGE_STEP = 10;
const INITIAL_RECT = { width: 800, height: 600 };

/**
 * Размер окна списка. У jsdom нет раскладки, а у скрытой вкладки размер нулевой: виртуализатор
 * тогда не отрисовал бы ни одной строки. Пока прямоугольник пустой, подставляем начальный размер;
 * настоящий придёт следующим вызовом ResizeObserver, когда окно станет видимым.
 */
const observeRect: typeof observeElementRect = (instance, cb) => observeElementRect(instance, (r) => cb(r.height > 0 && r.width > 0 ? r : INITIAL_RECT));

type Mark = "added" | "removed" | "changed";

const scanLabel = (s: ScanMeta) => `${s.at.slice(0, 16).replace("T", " ")} · ${s.service} · ${s.version}`;

/** Снимки разборов: счётчики по каждому и выбор двух снимков для диффа. */
function ScansSection({ scans, from, to, onFrom, onTo }: { scans: ScanMeta[]; from: string; to: string; onFrom: (f: string) => void; onTo: (f: string) => void }) {
  const { t } = useTranslation();
  return (
    <details className="panel">
      <summary>{t("registry.scans", { count: scans.length })}</summary>
      <div className="row">
        <label>
          {t("registry.diffFrom")}
          <select aria-label={t("registry.diffFrom")} value={from} onChange={(e) => onFrom(e.target.value)}>
            {scans.map((s) => (
              <option key={s.file} value={s.file}>
                {scanLabel(s)}
              </option>
            ))}
          </select>
        </label>
        <label>
          {t("registry.diffTo")}
          <select aria-label={t("registry.diffTo")} value={to} onChange={(e) => onTo(e.target.value)}>
            {scans.map((s) => (
              <option key={s.file} value={s.file}>
                {scanLabel(s)}
              </option>
            ))}
          </select>
        </label>
      </div>
      <ul className="scan-list">
        {scans.map((s) => (
          <li key={s.file}>
            <code>{scanLabel(s)}</code>{" "}
            <span className="muted">{t("registry.scanCounts", { added: s.added, changed: s.changed, removed: s.removed, restored: s.restored })}</span>
          </li>
        ))}
      </ul>
    </details>
  );
}

/** Таблицы из миграций по сервисам: подсказки для ручных sql-операций. Читаются при первом раскрытии. */
function TablesSection({ path }: { path: string }) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const tables = useQuery({ queryKey: ["tables", path], queryFn: () => api.registryTables(path), enabled: open });
  const services = Object.entries(tables.data ?? {});
  return (
    <details className="panel" onToggle={(e) => setOpen(e.currentTarget.open)}>
      <summary>{t("registry.tables")}</summary>
      {tables.error && <ErrorBanner error={tables.error} onRetry={() => tables.refetch()} />}
      {open && tables.data && services.length === 0 && <p className="muted">{t("registry.noTables")}</p>}
      {services.map(([service, list]) => (
        <section key={service}>
          <h4>{service}</h4>
          <ul className="schema-tree">
            {list.map((table) => (
              <li key={table.name}>
                <code>{table.name}</code>
                <ul>
                  {table.columns.map((c) => (
                    <li key={c.name}>
                      <code>{c.name}</code> <span className="muted">{c.dataType}</span>
                      {!c.nullable && <span className="warn" title={t("registry.notNull")}> *</span>}
                    </li>
                  ))}
                </ul>
              </li>
            ))}
          </ul>
        </section>
      ))}
    </details>
  );
}

export function RegistryPanel({ path }: { path: string }) {
  const { t } = useTranslation();
  useHelpChapter(tabId({ kind: "registry", path }), "registry");
  const [service, setService] = useState("");
  const [kind, setKind] = useState<OpKind | "">("");
  const [source, setSource] = useState<SourceKind | "">("");
  const [status, setStatus] = useState<Status | "">("active");
  const [query, setQuery] = useState("");
  const debounced = useDebouncedValue(query, 200);
  const [selected, setSelected] = useState<string | null>(null);
  const [adding, setAdding] = useState(false);
  const [diffOn, setDiffOn] = useState(false);
  const [fromPick, setFromPick] = useState("");
  const [toPick, setToPick] = useState("");
  const filter: RegistryFilter = useMemo(
    () => ({ service: service || undefined, kind: kind || undefined, source: source || undefined, status: status || undefined, query: debounced || undefined }),
    [service, kind, source, status, debounced],
  );
  const project = useQuery({ queryKey: ["project", path], queryFn: () => api.projectView(path) });
  const rows = useQuery({ queryKey: ["registry", path, filter], queryFn: () => api.registryList(path, filter) });
  const scans = useQuery({ queryKey: ["scans", path], queryFn: () => api.registryScans(path) });
  // Снимки идут от новых к старым; по умолчанию сравниваем два последних.
  const scanList = scans.data ?? [];
  const from = fromPick || scanList[1]?.file;
  const to = toPick || scanList[0]?.file;
  const fromScan = scanList.find((s) => s.file === from);
  const toScan = scanList.find((s) => s.file === to);
  const diff = useQuery({ queryKey: ["diff", path, from, to], queryFn: () => api.registryDiff(path, from!, to!), enabled: diffOn && !!from && !!to });
  const marks = useMemo(() => {
    const m = new Map<string, Mark>();
    if (!diffOn || !diff.data) return m;
    diff.data.added.forEach((id) => m.set(id, "added"));
    diff.data.removed.forEach((id) => m.set(id, "removed"));
    diff.data.changed.forEach((id) => m.set(id, "changed"));
    return m;
  }, [diffOn, diff.data]);
  const clones = useMemo(() => Object.fromEntries((project.data?.manifest.services ?? []).map((s) => [s.id, joinProjectPath(path, s.path)])), [project.data, path]);
  const parentRef = useRef<HTMLDivElement>(null);
  const list: OperationRow[] = rows.data ?? [];
  // Строка заголовков прилипает сверху окна списка: строки начинаются на её высоту ниже (scrollMargin),
  // а прокрутка к строке не прячет её под заголовком (scrollPaddingStart).
  const virtual = useVirtualizer({
    count: list.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => ROW_HEIGHT,
    overscan: 10,
    initialRect: INITIAL_RECT,
    observeElementRect: observeRect,
    scrollMargin: ROW_HEIGHT,
    scrollPaddingStart: ROW_HEIGHT,
  });
  const items = virtual.getVirtualItems();
  const selectedIndex = selected ? list.findIndex((r) => r.id === selected) : -1;
  // Единственная точка входа с Tab: выбранная строка, а если её не видно — первая из отрисованных.
  const tabStop = selectedIndex >= 0 && items.some((v) => v.index === selectedIndex) ? selectedIndex : (items[0]?.index ?? -1);

  // Фокус на строку, до которой дошли стрелками: она может появиться в DOM только после прокрутки.
  const pendingFocus = useRef<number | null>(null);
  useEffect(() => {
    if (pendingFocus.current === null) return;
    const el = parentRef.current?.querySelector<HTMLElement>(`[data-index="${pendingFocus.current}"]`);
    if (el) {
      el.focus();
      pendingFocus.current = null;
    }
  });
  const go = (index: number) => {
    const i = Math.max(0, Math.min(list.length - 1, index));
    if (list.length === 0) return;
    setSelected(list[i].id);
    pendingFocus.current = i;
    virtual.scrollToIndex(i);
  };
  const onRowKey = (e: KeyboardEvent, index: number) => {
    switch (e.key) {
      case "Enter":
      case " ":
        setSelected(list[index].id);
        break;
      case "ArrowDown":
        go(index + 1);
        break;
      case "ArrowUp":
        go(index - 1);
        break;
      case "PageDown":
        go(index + PAGE_STEP);
        break;
      case "PageUp":
        go(index - PAGE_STEP);
        break;
      case "Home":
        go(0);
        break;
      case "End":
        go(list.length - 1);
        break;
      default:
        return;
    }
    e.preventDefault();
  };

  return (
    <div className="page registry">
      <div className="toolbar">
        <h2>
          {t("registry.title")} <Hint k="hints.registry.title" label={t("registry.title")} />
        </h2>
        <input aria-label={t("registry.search")} placeholder={t("registry.search")} value={query} onChange={(e) => setQuery(e.target.value)} />
        <select aria-label={t("registry.service")} value={service} onChange={(e) => setService(e.target.value)}>
          <option value="">{t("registry.anyService")}</option>
          {(project.data?.manifest.services ?? []).map((s) => (
            <option key={s.id} value={s.id}>
              {s.id}
            </option>
          ))}
        </select>
        <select aria-label={t("registry.kind")} value={kind} onChange={(e) => setKind(e.target.value as OpKind | "")}>
          <option value="">{t("registry.anyKind")}</option>
          {KINDS.map((k) => (
            <option key={k} value={k}>
              {t(`registry.kinds.${k}`)}
            </option>
          ))}
        </select>
        <select aria-label={t("registry.sourceLabel")} value={source} onChange={(e) => setSource(e.target.value as SourceKind | "")}>
          <option value="">{t("registry.anySource")}</option>
          {SOURCES.map((s) => (
            <option key={s} value={s}>
              {s}
            </option>
          ))}
        </select>
        <select aria-label={t("registry.statusLabel")} value={status} onChange={(e) => setStatus(e.target.value as Status | "")}>
          <option value="">{t("registry.anyStatus")}</option>
          <option value="active">{t("registry.status.active")}</option>
          <option value="removed">{t("registry.status.removed")}</option>
        </select>
        <label>
          <input
            type="checkbox"
            checked={diffOn}
            disabled={!from || !to}
            onChange={(e) => {
              setDiffOn(e.target.checked);
              // Удалённые операции по умолчанию скрыты фильтром «активные» — в диффе их отметки должны быть видны.
              if (e.target.checked) setStatus("");
            }}
          />{" "}
          {t("registry.diffToggle")}
        </label>
        <button type="button" onClick={() => setAdding(true)} disabled={!project.data || project.data.readOnly}>
          {t("registry.addManual")}
        </button>
        <span className="muted">{t("registry.count", { count: list.length })}</span>
      </div>
      {diffOn && fromScan && toScan && <p className="muted">{t("registry.diffBetween", { from: fromScan.at.slice(0, 16), to: toScan.at.slice(0, 16) })}</p>}
      {diff.error && <ErrorBanner error={diff.error} onRetry={() => diff.refetch()} />}
      {rows.error && <ErrorBanner error={rows.error} onRetry={() => rows.refetch()} />}
      {project.error && <ErrorBanner error={project.error} onRetry={() => project.refetch()} />}
      {adding && project.data && (
        <ManualOperationForm
          path={path}
          services={project.data.manifest.services.map((s) => s.id)}
          onDone={(op) => {
            setAdding(false);
            setSelected(op.id);
            void rows.refetch();
          }}
          onCancel={() => setAdding(false)}
        />
      )}
      <div className="registry-body">
        <div className="vlist" ref={parentRef} role="grid" aria-label={t("registry.title")} aria-rowcount={list.length + 1}>
          <div role="row" aria-rowindex={1} className="msg-row msg-head" style={{ height: ROW_HEIGHT }}>
            <span role="columnheader">ID</span>
            <span role="columnheader">{t("registry.service")}</span>
            <span role="columnheader">{t("registry.kind")}</span>
            <span role="columnheader">{t("registry.name")}</span>
            <span role="columnheader">{t("registry.sourceLabel")}</span>
            <span role="columnheader">{t("registry.version")}</span>
            <span role="columnheader">{t("registry.change")}</span>
          </div>
          <div style={{ height: virtual.getTotalSize(), position: "relative" }}>
            {items.map((v) => {
              const r = list[v.index];
              if (!r) return null;
              const mark = marks.get(r.id);
              return (
                <div
                  key={r.id}
                  role="row"
                  aria-rowindex={v.index + 2}
                  aria-selected={selected === r.id}
                  data-index={v.index}
                  tabIndex={v.index === tabStop ? 0 : -1}
                  className={["msg-row", selected === r.id ? "active" : "", mark ? `diff-${mark}` : "", r.status === "removed" ? "muted" : ""].join(" ")}
                  style={{ position: "absolute", top: v.start - ROW_HEIGHT, height: ROW_HEIGHT, width: "100%" }}
                  onClick={() => setSelected(r.id)}
                  onKeyDown={(e) => onRowKey(e, v.index)}
                >
                  <span role="gridcell" title={r.id}>
                    <code>{r.id}</code>
                    {r.topicGuessed && (
                      <>
                        {" "}
                        <span className="guessed" role="img" title={t("registry.topicGuessed")} aria-label={t("registry.topicGuessed")}>
                          ?
                        </span>
                      </>
                    )}
                  </span>
                  <span role="gridcell">{r.service}</span>
                  <span role="gridcell">
                    <span className={`kind-badge kind-${r.kind}`}>{t(`registry.kinds.${r.kind}`)}</span>
                  </span>
                  <span role="gridcell" className="muted">{r.name}</span>
                  <span role="gridcell" className="muted">{r.source}</span>
                  <span role="gridcell" className="muted">{r.lastVersion}</span>
                  <span role="gridcell">{mark && <span className={`sev sev-${mark}`}>{t(`registry.marks.${mark}`)}</span>}</span>
                </div>
              );
            })}
          </div>
          {rows.data && list.length === 0 && <p className="empty">{t("registry.empty")}</p>}
        </div>
        {selected && <OperationDetails key={selected} path={path} id={selected} clones={clones} readOnly={project.data?.readOnly} onDeleted={() => setSelected(null)} />}
      </div>
      {scanList.length > 0 && <ScansSection scans={scanList} from={from ?? ""} to={to ?? ""} onFrom={setFromPick} onTo={setToPick} />}
      <TablesSection path={path} />
    </div>
  );
}
