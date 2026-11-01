import { useMutation, useQuery } from "@tanstack/react-query";
import { Fragment, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../api/client";
import type { Agent, AgentProbeState, AgentStatus } from "../../api/types";
import { ErrorBanner } from "../common/ErrorBanner";
import { Hint } from "../common/Hint";
import { NumberInput } from "../common/NumberInput";

/** Формат ID агента — как у сервисов и стендов (`is_id` в Rust). */
export const AGENT_ID_RE = /^[a-z0-9-]{1,64}$/;
/** Rust проверяет только схему http или https в начале; здесь ещё и непустой адрес без пробелов после неё. */
const URL_RE = /^https?:\/\/\S+$/;
/** `weight` в Rust — `u32`. */
const MAX_WEIGHT = 0xffffffff;

export const validAgentUrl = (url: string): boolean => URL_RE.test(url);
export const validAgentWeight = (w: number): boolean => Number.isInteger(w) && w >= 1 && w <= MAX_WEIGHT;

/** Цвет бейджа probe: зелёный — готов, жёлтый — занят или без токена, красный — недоступен или другая версия. */
const BADGE: Record<AgentProbeState, "ok" | "warn" | "err"> = { ready: "ok", busy: "warn", no_token: "warn", unreachable: "err", version: "err" };

/** Бейдж состояния агента по ответу `agent_probe` с подробностью: версия и ядра, id чужого прогона или причина ошибки. */
export function AgentStateBadge({ status }: { status: AgentStatus }) {
  const { t } = useTranslation();
  const detail =
    status.state === "ready" && status.info
      ? t("agents.info", { version: status.info.version, os: status.info.os, arch: status.info.arch, count: status.info.cpus })
      : status.state === "busy" && status.info?.busy
        ? t("agents.busyRun", { run: status.info.busy })
        : (status.message ?? "");
  return (
    <span className="agent-status">
      <span className={`state-badge state-${BADGE[status.state]}`} data-testid="agent-state">
        {t(`agents.state.${status.state}`)}
      </span>
      {detail && <span className="muted"> {detail}</span>}
    </span>
  );
}

/**
 * Поле токена: значение уходит только в `agent_set_token` (хранилище ОС) и не попадает в манифест.
 * `agent_set_token` ищет агента в сохранённом манифесте — новому агенту токен задаётся после сохранения.
 */
function TokenField({
  path,
  envId,
  agent,
  status,
  disabled,
  unsaved,
  onSaved,
}: {
  path: string;
  envId: string;
  agent: Agent;
  status: AgentStatus | undefined;
  disabled: boolean;
  unsaved: boolean;
  onSaved: () => void;
}) {
  const { t } = useTranslation();
  const [token, setToken] = useState("");
  const [last, setLast] = useState<"saved" | "removed" | null>(null);
  const set = useMutation({
    mutationFn: (value: string | null) => api.agentSetToken(path, envId, agent.id, value),
    onSuccess: (_r, value) => {
      setToken("");
      setLast(value === null ? "removed" : "saved");
      onSaved();
    },
  });
  const off = disabled || unsaved;
  const indicator = unsaved ? (
    <span className="muted">{t("agents.saveFirst")}</span>
  ) : last === "saved" ? (
    <span className="muted">{t("agents.tokenSaved")}</span>
  ) : last === "removed" ? (
    <span className="muted">{t("agents.tokenRemoved")}</span>
  ) : status?.state === "no_token" ? (
    <span className="warn">{t("agents.tokenMissing")}</span>
  ) : status ? (
    <span className="muted">{t("agents.tokenSet")}</span>
  ) : null;
  return (
    <div className="row">
      <input
        type="password"
        autoComplete="new-password"
        aria-label={t("agents.token", { id: agent.id })}
        placeholder={t("agents.tokenPlaceholder")}
        value={token}
        disabled={off}
        onChange={(e) => setToken(e.target.value)}
      />
      <button type="button" disabled={off || !token.trim() || set.isPending} onClick={() => set.mutate(token)}>
        {t("agents.saveToken")}
      </button>
      <button type="button" disabled={off || set.isPending} onClick={() => set.mutate(null)}>
        {t("agents.removeToken")}
      </button>
      {indicator}
      {set.error && <ErrorBanner error={set.error} />}
    </div>
  );
}

/**
 * Агенты стенда в форме проекта: таблица id/URL/вес, токен каждого (в хранилище ОС) и «Проверить» —
 * `agent_probe` по агентам сохранённого манифеста. `saved` — ID агентов этого стенда в сохранённом манифесте.
 */
export function EnvironmentAgents({
  path,
  envId,
  agents,
  saved,
  onChange,
  disabled = false,
}: {
  path: string;
  envId: string;
  agents: Agent[];
  saved: string[];
  onChange: (agents: Agent[]) => void;
  disabled?: boolean;
}) {
  const { t } = useTranslation();
  const canProbe = saved.length > 0;
  // Только по кнопке: probe ходит по сети, вкладка проекта не должна делать этого сама.
  const probe = useQuery({ queryKey: ["agentProbe", path, envId], queryFn: () => api.agentProbe(path, envId), enabled: false });
  const statuses = new Map((probe.data ?? []).map((s) => [s.id, s]));
  const ids = agents.map((a) => a.id);
  // Устойчивые ключи строк: не зависят ни от индекса, ни от редактируемого id, поэтому введённый токен
  // остаётся у своего агента при удалении соседних строк. Ключи идут параллельно `agents`: добавление и
  // удаление правят их здесь же. Если список подменили снаружи (не наши add/remove/update) — выдаём все ключи
  // заново, чтобы ни одна строка не унаследовала состояние чужой.
  const rowKeys = useRef<number[]>([]);
  const nextKey = useRef(0);
  const known = useRef<Agent[] | null>(null);
  if (known.current !== agents) {
    if (known.current !== null) rowKeys.current = [];
    known.current = agents;
  }
  while (rowKeys.current.length < agents.length) rowKeys.current.push(nextKey.current++);
  rowKeys.current.length = agents.length;
  const emit = (next: Agent[]) => {
    known.current = next;
    onChange(next);
  };
  const update = (i: number, patch: Partial<Agent>) => emit(agents.map((a, j) => (j === i ? { ...a, ...patch } : a)));
  const add = () => {
    rowKeys.current.push(nextKey.current++);
    emit([...agents, { id: "", url: "", weight: 1 }]);
  };
  const remove = (i: number) => {
    rowKeys.current.splice(i, 1);
    emit(agents.filter((_, j) => j !== i));
  };
  return (
    <section className="agents">
      <h4>
        {t("agents.title")} <Hint k="hints.project.agents" label={t("agents.title")} />
      </h4>
      {agents.length === 0 && <p className="muted">{t("agents.empty")}</p>}
      {agents.length > 0 && (
        <table>
          <thead>
            <tr>
              <th>{t("agents.id")}</th>
              <th>{t("agents.url")}</th>
              <th>{t("agents.weight")}</th>
              <th>{t("agents.status")}</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {agents.map((a, i) => {
              const badId = !AGENT_ID_RE.test(a.id) || ids.indexOf(a.id) !== i;
              const status = statuses.get(a.id);
              return (
                <Fragment key={rowKeys.current[i]}>
                  <tr>
                    <td>
                      <input aria-label={t("agents.id")} aria-invalid={badId || undefined} value={a.id} disabled={disabled} onChange={(e) => update(i, { id: e.target.value })} />
                    </td>
                    <td>
                      <input
                        aria-label={t("agents.url")}
                        aria-invalid={!validAgentUrl(a.url) || undefined}
                        placeholder="http://10.0.1.11:7420"
                        value={a.url}
                        disabled={disabled}
                        onChange={(e) => update(i, { url: e.target.value })}
                      />
                    </td>
                    <td>
                      <NumberInput
                        aria-label={t("agents.weight")}
                        aria-invalid={!validAgentWeight(a.weight) || undefined}
                        min={1}
                        step={1}
                        value={a.weight}
                        disabled={disabled}
                        onChange={(v) => update(i, { weight: v ?? Number.NaN })}
                      />
                    </td>
                    <td>{status ? <AgentStateBadge status={status} /> : <span className="muted">—</span>}</td>
                    <td>
                      <button type="button" className="icon" aria-label={t("agents.remove", { id: a.id })} disabled={disabled} onClick={() => remove(i)}>
                        ×
                      </button>
                    </td>
                  </tr>
                  <tr>
                    <td colSpan={5}>
                      <TokenField
                        path={path}
                        envId={envId}
                        agent={a}
                        status={status}
                        disabled={disabled || badId}
                        unsaved={!badId && !saved.includes(a.id)}
                        onSaved={() => probe.data && void probe.refetch()}
                      />
                    </td>
                  </tr>
                </Fragment>
              );
            })}
          </tbody>
        </table>
      )}
      <div className="row">
        <button type="button" disabled={disabled} onClick={add}>
          {t("agents.add")}
        </button>
        <button type="button" disabled={!canProbe || probe.isFetching} onClick={() => void probe.refetch()}>
          {t("agents.probe")}
        </button>
        <span className="muted">{t("agents.probeSavedOnly")}</span>
      </div>
      {probe.error && <ErrorBanner error={probe.error} onRetry={() => void probe.refetch()} />}
    </section>
  );
}
