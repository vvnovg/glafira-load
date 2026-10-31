import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../api/client";
import type { ContractKind, ContractRef, Direction, Manifest, Project, ScanProgress, ScanReport, Service, TopicSchema } from "../../api/types";
import { joinProjectPath, projectRelative } from "../../lib/paths";
import { useHelpChapter } from "../../state/helpContext";
import { tabId } from "../../state/workspace";
import { ErrorBanner } from "../common/ErrorBanner";
import { Hint } from "../common/Hint";
import { ScanReportView } from "./ScanReportView";

const KINDS: ContractKind[] = ["openapi", "proto", "asyncapi", "schema", "sql", "spring"];
const sameContract = (a: ContractRef, b: ContractRef) => a.kind === b.kind && a.path === b.path;

/** Контракт в виде для манифеста: `enabled: true` и пустой `include` не записываются (как в serde). */
function normalize(c: ContractRef): ContractRef {
  const { enabled, include, ...rest } = c;
  return { ...rest, ...(include && include.length > 0 ? { include } : {}), ...(enabled === false ? { enabled } : {}) };
}

const parseList = (s: string) => s.split(",").map((x) => x.trim()).filter(Boolean);

/** `onChange` возвращает false, если правка отклонена (такой вид и путь уже есть в списке). */
function ContractRow({ c, clone, project, disabled, onChange, onRemove }: { c: ContractRef; clone: string; project: string; disabled: boolean; onChange: (c: ContractRef) => boolean; onRemove: () => void }) {
  const { t } = useTranslation();
  const [path, setPath] = useState(c.path);
  const [include, setInclude] = useState((c.include ?? []).join(", "));
  // Правка дала повтор другой строки: строка помечается, манифест не сохраняется.
  const [duplicate, setDuplicate] = useState(false);
  const openFile = useMutation({ mutationFn: () => api.openPath(project, joinProjectPath(clone, c.path)) });
  const commitPath = () => {
    const next = path.trim();
    if (next && next !== c.path) setDuplicate(!onChange({ ...c, path: next }));
    else {
      setPath(c.path);
      setDuplicate(false);
    }
  };
  const commitInclude = () => {
    const next = parseList(include);
    if (next.join("\n") !== (c.include ?? []).join("\n")) void onChange({ ...c, include: next });
  };
  return (
    <li className="row">
      <input type="checkbox" aria-label={c.path} checked={c.enabled ?? true} disabled={disabled} onChange={(e) => void onChange({ ...c, enabled: e.target.checked })} />
      <select aria-label={t("service.kind", { path: c.path })} aria-invalid={duplicate || undefined} value={c.kind} disabled={disabled} onChange={(e) => setDuplicate(!onChange({ ...c, kind: e.target.value as ContractKind }))}>
        {KINDS.map((k) => (
          <option key={k} value={k}>
            {k}
          </option>
        ))}
      </select>
      <input aria-label={t("service.path", { path: c.path })} aria-invalid={duplicate || undefined} value={path} disabled={disabled} onChange={(e) => setPath(e.target.value)} onBlur={commitPath} />
      <input aria-label={t("service.include", { path: c.path })} placeholder={t("service.includePlaceholder")} value={include} disabled={disabled} onChange={(e) => setInclude(e.target.value)} onBlur={commitInclude} />
      <button type="button" className="link" onClick={() => openFile.mutate()}>
        {t("service.openFile")}
      </button>
      <button type="button" className="icon" aria-label={t("service.removeContract", { path: c.path })} disabled={disabled} onClick={onRemove}>
        ×
      </button>
      {duplicate && <span className="warn">{t("service.duplicateContract")}</span>}
      {openFile.error && <ErrorBanner error={openFile.error} />}
    </li>
  );
}

function AddContract({ clone, disabled, onAdd }: { clone: string; disabled: boolean; onAdd: (c: ContractRef) => void }) {
  const { t } = useTranslation();
  const [kind, setKind] = useState<ContractKind>("openapi");
  const [path, setPath] = useState("");
  const choose = async () => {
    const file = await openDialog({ directory: false, multiple: false, defaultPath: clone, title: t("service.chooseFile") });
    if (typeof file === "string") setPath(projectRelative(clone, file));
  };
  return (
    <div className="row">
      <select aria-label={t("service.newKind")} value={kind} onChange={(e) => setKind(e.target.value as ContractKind)}>
        {KINDS.map((k) => (
          <option key={k} value={k}>
            {k}
          </option>
        ))}
      </select>
      <input aria-label={t("service.newPath")} value={path} onChange={(e) => setPath(e.target.value)} />
      <button type="button" onClick={choose}>
        {t("service.chooseFile")}
      </button>
      <button
        type="button"
        disabled={disabled || !path.trim()}
        onClick={() => {
          onAdd({ kind, path: path.trim() });
          setPath("");
        }}
      >
        {t("service.addContract")}
      </button>
    </div>
  );
}

const topicSchema = (v: TopicSchema) => (typeof v === "string" ? v : v.schema);
const topicDirection = (v: TopicSchema): Direction | "" => (typeof v === "string" ? "" : (v.direction ?? ""));
const makeTopic = (schema: string, direction: Direction | ""): TopicSchema => (direction ? { schema, direction } : schema);

function TopicRow({ topic, value, disabled, onChange, onRemove }: { topic: string; value: TopicSchema; disabled: boolean; onChange: (v: TopicSchema) => void; onRemove: () => void }) {
  const { t } = useTranslation();
  const [schema, setSchema] = useState(topicSchema(value));
  const direction = topicDirection(value);
  return (
    <li className="row">
      <code>{topic}</code>
      <input
        aria-label={t("service.topicSchema", { topic })}
        value={schema}
        disabled={disabled}
        onChange={(e) => setSchema(e.target.value)}
        onBlur={() => (schema.trim() !== topicSchema(value) ? onChange(makeTopic(schema.trim(), direction)) : setSchema(topicSchema(value)))}
      />
      <select aria-label={t("service.topicDirection", { topic })} value={direction} disabled={disabled} onChange={(e) => onChange(makeTopic(topicSchema(value), e.target.value as Direction | ""))}>
        <option value="">{t("service.directionAuto")}</option>
        <option value="produce">produce</option>
        <option value="consume">consume</option>
      </select>
      <button type="button" className="icon" aria-label={t("service.removeTopic", { topic })} disabled={disabled} onClick={onRemove}>
        ×
      </button>
    </li>
  );
}

function KafkaTopics({ topics, disabled, onChange }: { topics: Record<string, TopicSchema>; disabled: boolean; onChange: (t: Record<string, TopicSchema>) => void }) {
  const { t } = useTranslation();
  const [topic, setTopic] = useState("");
  const [schema, setSchema] = useState("");
  const [direction, setDirection] = useState<Direction | "">("");
  const entries = Object.entries(topics);
  return (
    <>
      <h3>
        {t("service.kafkaTopics")} <Hint k="hints.service.kafka" label={t("service.kafkaTopics")} />
      </h3>
      {entries.length === 0 && <p className="muted">{t("service.noTopics")}</p>}
      <ul className="tree">
        {entries.map(([name, v]) => (
          <TopicRow
            key={`${name}:${topicSchema(v)}`}
            topic={name}
            value={v}
            disabled={disabled}
            onChange={(next) => onChange({ ...topics, [name]: next })}
            onRemove={() => onChange(Object.fromEntries(entries.filter(([n]) => n !== name)))}
          />
        ))}
      </ul>
      <div className="row">
        <input aria-label={t("service.newTopic")} value={topic} onChange={(e) => setTopic(e.target.value)} />
        <input aria-label={t("service.newTopicSchema")} value={schema} onChange={(e) => setSchema(e.target.value)} />
        <select aria-label={t("service.newTopicDirection")} value={direction} onChange={(e) => setDirection(e.target.value as Direction | "")}>
          <option value="">{t("service.directionAuto")}</option>
          <option value="produce">produce</option>
          <option value="consume">consume</option>
        </select>
        <button
          type="button"
          disabled={disabled || !topic.trim() || !schema.trim()}
          onClick={() => {
            onChange({ ...topics, [topic.trim()]: makeTopic(schema.trim(), direction) });
            setTopic("");
            setSchema("");
            setDirection("");
          }}
        >
          {t("service.addTopic")}
        </button>
      </div>
    </>
  );
}

export function ServicePanel({ path, serviceId }: { path: string; serviceId: string }) {
  const { t } = useTranslation();
  const qc = useQueryClient();
  useHelpChapter(tabId({ kind: "service", path, serviceId }), "project");
  const project = useQuery({ queryKey: ["project", path], queryFn: () => api.projectView(path) });
  const service = project.data?.manifest.services.find((s) => s.id === serviceId);
  const missing = project.data?.missingServices.includes(serviceId) ?? false;
  const discovered = useQuery({ queryKey: ["contracts", path, serviceId], queryFn: () => api.serviceDiscoverContracts(path, serviceId), enabled: !!service && !missing });
  const reportKey = ["service-scan", path, serviceId];
  const [progress, setProgress] = useState<ScanProgress | null>(null);
  // Последний отчёт переживает закрытие и повторное открытие вкладки: лежит в кэше запросов.
  const [report, setReport] = useState<ScanReport | null>(() => qc.getQueryData<ScanReport>(reportKey) ?? null);
  const save = useMutation({
    mutationFn: (m: Manifest) => api.projectSave(path, m),
    onSuccess: (p) => {
      qc.setQueryData(["project", path], p);
      void qc.invalidateQueries({ queryKey: ["projects"] });
    },
  });
  const scan = useMutation({
    mutationFn: () => api.serviceScan(path, serviceId, setProgress),
    onSuccess: (r) => {
      const next = r[0] ?? null;
      setReport(next);
      qc.setQueryData(reportKey, next ?? undefined);
      setProgress(null);
      void qc.invalidateQueries({ queryKey: ["registry", path] });
      void qc.invalidateQueries({ queryKey: ["scans", path] });
    },
    onError: () => setProgress(null),
  });
  const cancel = useMutation({ mutationFn: () => api.scanCancel(path, serviceId) });
  const openFolder = useMutation({ mutationFn: (dir: string) => api.openPath(path, dir) });
  const discover = useMutation({ mutationFn: () => api.serviceDiscoverContracts(path, serviceId) });

  if (project.error) return <ErrorBanner error={project.error} onRetry={() => project.refetch()} />;
  if (!project.data) return <p className="empty">{t("common.loading")}</p>;
  if (!service) return <p className="empty">{t("service.notInManifest", { id: serviceId })}</p>;
  const manifest = project.data.manifest;
  const readOnly = project.data.readOnly;
  const clone = joinProjectPath(path, service.path);
  // Список контрактов: явный из манифеста, иначе найденный автопоиском (все включены).
  const explicit = service.contracts.length > 0;
  const contracts: ContractRef[] = explicit ? service.contracts : (discovered.data ?? []);
  const editDisabled = save.isPending || readOnly;

  // Правка накладывается на последний манифест из кэша: после await (поиск контрактов) снимок рендера мог устареть.
  const latestManifest = () => qc.getQueryData<Project>(["project", path])?.manifest ?? manifest;
  const saveService = (patch: (s: Service) => Service) => {
    const base = latestManifest();
    save.mutate({ ...base, services: base.services.map((s) => (s.id === serviceId ? patch(s) : s)) });
  };
  const writeContracts = (next: ContractRef[]) => saveService((s) => ({ ...s, contracts: next.map(normalize) }));
  const writeTopics = (topics: Record<string, TopicSchema>) =>
    saveService((s) => {
      const next: Service = { ...s };
      if (Object.keys(topics).length > 0) next.kafka = { topics };
      else delete next.kafka;
      return next;
    });
  /** Правка строки; повтор вида и пути другой строки не сохраняется (false). */
  const change = (i: number, c: ContractRef): boolean => {
    if (contracts.some((x, j) => j !== i && sameContract(x, c))) return false;
    writeContracts(contracts.map((x, j) => (j === i ? c : x)));
    return true;
  };
  const mergeDiscovered = async () => {
    const found = await discover.mutateAsync();
    // Список берём из свежего манифеста: пока шёл поиск, его могли сохранить другие правки.
    const current = latestManifest().services.find((s) => s.id === serviceId)?.contracts ?? [];
    const fresh = found.filter((f) => !current.some((x) => sameContract(x, f)));
    // Нового нет — манифест не трогаем (и не пишем пустой список, если не нашлось ничего).
    if (fresh.length === 0) return;
    writeContracts([...current, ...fresh]);
  };

  return (
    <div className="page">
      <div className="toolbar">
        <h2>{service.name || service.id}</h2>
        <button type="button" className="primary" disabled={scan.isPending || missing || readOnly} onClick={() => scan.mutate()}>
          {t("service.scan")}
        </button>
        {scan.isPending && (
          <button type="button" disabled={cancel.isPending} onClick={() => cancel.mutate()}>
            {t("common.cancel")}
          </button>
        )}
        <button type="button" onClick={() => openFolder.mutate(clone)} disabled={missing}>
          {t("project.openFolder")}
        </button>
      </div>
      <p>
        <code>{clone}</code> {missing && <span className="warn">{t("project.serviceMissing")}</span>}
      </p>
      {openFolder.error && <ErrorBanner error={openFolder.error} />}
      {cancel.error && <ErrorBanner error={cancel.error} />}
      <h3>
        {t("service.contracts")} <Hint k="hints.service.contracts" label={t("service.contracts")} />
      </h3>
      {discovered.error && <ErrorBanner error={discovered.error} onRetry={() => discovered.refetch()} />}
      {!explicit && discovered.isPending && !missing && <p className="muted">{t("common.loading")}</p>}
      {contracts.length === 0 && !discovered.isPending && <p className="empty">{t("service.noContracts")}</p>}
      <ul className="tree">
        {contracts.map((c, i) => (
          <ContractRow key={`${c.kind}:${c.path}`} c={c} clone={clone} project={path} disabled={editDisabled} onChange={(next) => change(i, next)} onRemove={() => writeContracts(contracts.filter((_, j) => j !== i))} />
        ))}
      </ul>
      {!explicit && contracts.length > 0 && <p className="muted">{t("service.autoFound")}</p>}
      <AddContract clone={clone} disabled={editDisabled} onAdd={(c) => !contracts.some((x) => sameContract(x, c)) && writeContracts([...contracts, c])} />
      <div className="row">
        <button type="button" disabled={editDisabled || missing || discover.isPending} onClick={() => void mergeDiscovered().catch(() => undefined)}>
          {t("service.discover")}
        </button>
      </div>
      {discover.error && <ErrorBanner error={discover.error} />}
      <KafkaTopics topics={service.kafka?.topics ?? {}} disabled={editDisabled} onChange={writeTopics} />
      {save.error && <ErrorBanner error={save.error} />}
      {progress && <p className="muted">{t("project.scanProgress", { service: progress.service, done: progress.done, total: progress.total, file: progress.file })}</p>}
      {scan.error && <ErrorBanner error={scan.error} />}
      {report && <ScanReportView report={report} project={path} clone={clone} />}
    </div>
  );
}
