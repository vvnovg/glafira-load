import { useQuery } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { api } from "../../api/client";
import type { AgentRunState, Environment } from "../../api/types";
import { ErrorBanner } from "../common/ErrorBanner";
import { AgentStateBadge } from "../project/EnvironmentAgents";

const LIVE_BADGE: Record<AgentRunState, "ok" | "warn" | "err"> = { ready: "ok", running: "ok", done: "ok", lost: "err", failed: "err" };

function LiveBadge({ state }: { state: AgentRunState }) {
  const { t } = useTranslation();
  return (
    <span className={`state-badge state-${LIVE_BADGE[state]}`} data-testid="agent-live">
      {t(`run.agentState.${state}`)}
    </span>
  );
}

/**
 * Агенты стенда: probe при открытии и по кнопке; во время прогона — состояние из событий `agent`.
 * Стенд без агентов гоняет прогон на встроенном агенте приложения (`local`) — его состояние тоже из событий.
 */
export function AgentsStatus({ path, env, live }: { path: string; env: Environment; live: Record<string, AgentRunState> }) {
  const { t } = useTranslation();
  const agents = env.agents ?? [];
  const probe = useQuery({ queryKey: ["agentProbe", path, env.id], queryFn: () => api.agentProbe(path, env.id), enabled: agents.length > 0 });
  const statuses = new Map((probe.data ?? []).map((s) => [s.id, s]));
  // Агенты из событий, которых нет в манифесте стенда (встроенный `local`).
  const extra = Object.entries(live).filter(([id]) => !agents.some((a) => a.id === id));
  return (
    <section>
      <div className="row">
        <h4>{t("run.agents")}</h4>
        {agents.length > 0 && (
          <button type="button" disabled={probe.isFetching} onClick={() => void probe.refetch()}>
            {t("agents.probe")}
          </button>
        )}
      </div>
      {agents.length === 0 && <p className="muted">{t("run.localAgent")}</p>}
      {probe.error && <ErrorBanner error={probe.error} onRetry={() => void probe.refetch()} />}
      <ul className="agent-list">
        {agents.map((a) => {
          const l = live[a.id];
          const s = statuses.get(a.id);
          return (
            <li key={a.id}>
              <code>{a.id}</code> <span className="muted">{a.url} · ×{a.weight}</span>{" "}
              {l ? <LiveBadge state={l} /> : s ? <AgentStateBadge status={s} /> : probe.isFetching ? <span className="muted">{t("common.loading")}</span> : null}
            </li>
          );
        })}
        {extra.map(([id, state]) => (
          <li key={id}>
            <code>{id}</code> <LiveBadge state={state} />
          </li>
        ))}
      </ul>
    </section>
  );
}
