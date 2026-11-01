import { useMutation, useQuery } from "@tanstack/react-query";
import { useEffect, useId, useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../api/client";
import type { Finding, Mode } from "../../api/types";
import { joinProjectPath } from "../../lib/paths";
import { isActive, useProjectRun, useRunStore } from "../../state/run";
import { ErrorBanner } from "../common/ErrorBanner";
import { Hint } from "../common/Hint";
import { NumberInput } from "../common/NumberInput";
import { CheckReport, hasErrors } from "../suites/CheckReport";
import { AgentsStatus } from "./AgentsStatus";

/** Потолок одновременности — целое ≥ 1, как `u32` в Rust; пусто — по умолчанию из сьюта. */
const MAX_IN_FLIGHT = 4_294_967_295;

/** Форма прогона: сьют, профиль, стенд, режим, потолок одновременности; «Запустить» сначала выполняет `check`. */
export function RunForm({ path }: { path: string }) {
  const { t } = useTranslation();
  const uid = useId();
  const run = useProjectRun(path);
  const start = useRunStore((s) => s.start);
  const stop = useRunStore((s) => s.stop);
  const reset = useRunStore((s) => s.reset);
  const project = useQuery({ queryKey: ["project", path], queryFn: () => api.projectView(path) });
  const suites = useQuery({ queryKey: ["suites", path], queryFn: () => api.suiteList(path) });
  const profiles = useQuery({ queryKey: ["profile", path], queryFn: () => api.profileGet(path) });
  // Вкладку закрыли и открыли во время прогона — поля показывают параметры идущего прогона.
  const [suite, setSuite] = useState(run.params?.suite ?? "");
  const [profile, setProfile] = useState(run.params?.profile ?? "");
  const [env, setEnv] = useState(run.params?.env ?? "");
  const [mode, setMode] = useState<Mode>(run.params?.mode ?? "run");
  const [maxInFlight, setMaxInFlight] = useState<number | undefined>(run.params?.maxInFlight ?? undefined);
  const [findings, setFindings] = useState<Finding[] | null>(null);
  // По умолчанию — первый сьют, профиль и стенд проекта.
  useEffect(() => {
    if (!suite && suites.data && suites.data.length > 0) setSuite(suites.data[0].name);
  }, [suites.data, suite]);
  useEffect(() => {
    const names = Object.keys(profiles.data?.profiles ?? {});
    if (!profile && names.length > 0) setProfile(names[0]);
  }, [profiles.data, profile]);
  useEffect(() => {
    const envs = project.data?.manifest.environments ?? [];
    if (!env && envs.length > 0) setEnv(envs[0].id);
  }, [project.data, env]);
  const check = useMutation({
    mutationFn: async () => (await api.suiteCheck(path, suite)).findings,
    onSuccess: setFindings,
  });
  const launch = useMutation({
    mutationFn: async () => {
      const found = (await api.suiteCheck(path, suite)).findings;
      setFindings(found);
      if (hasErrors(found)) return;
      // Цель RPS для графика фронтенд считает по таймлайну и сумме целей профиля — свежий расчёт на момент запуска.
      const computed = mode === "run" ? await api.profileCompute(path, profile) : null;
      await start(path, { suite, profile, env, mode, maxInFlight: maxInFlight ?? null, timeline: computed?.timeline ?? [], total: computed?.total ?? 0 });
    },
  });
  const busy = run.status === "failed" && run.error?.code === "RUN_BUSY";
  const stopRun = useMutation({
    mutationFn: async () => {
      await stop(path);
      // Идущий прогон проекта (его запустили не с этой формы) остановлен — форма снова свободна.
      if (busy) reset(path);
    },
  });
  const openFile = useMutation({ mutationFn: (file: string) => api.openPath(path, joinProjectPath(path, file)) });
  const active = isActive(run.status);
  const blocked = findings !== null && hasErrors(findings);
  const maxInvalid = maxInFlight !== undefined && !(Number.isInteger(maxInFlight) && maxInFlight >= 1 && maxInFlight <= MAX_IN_FLIGHT);
  const envs = project.data?.manifest.environments ?? [];
  const selectedEnv = envs.find((e) => e.id === env);
  const canStart = !active && !busy && !blocked && !!suite && !!profile && !!env && !maxInvalid && !launch.isPending;
  return (
    <section className="form stack run-form">
      <div className="row">
        <label>
          {t("run.suite")}
          <select
            value={suite}
            disabled={active}
            onChange={(e) => {
              setSuite(e.target.value);
              setFindings(null);
            }}
          >
            {(suites.data ?? []).map((s) => (
              <option key={s.name} value={s.name}>
                {s.name}
              </option>
            ))}
          </select>
        </label>
        <label>
          {t("run.profile")}
          <select value={profile} disabled={active} onChange={(e) => setProfile(e.target.value)}>
            {Object.keys(profiles.data?.profiles ?? {}).map((n) => (
              <option key={n} value={n}>
                {n}
              </option>
            ))}
          </select>
        </label>
        <label>
          {t("run.env")}
          <select value={env} disabled={active} onChange={(e) => setEnv(e.target.value)}>
            {envs.map((e) => (
              <option key={e.id} value={e.id}>
                {e.id}
              </option>
            ))}
          </select>
        </label>
        <label>
          {t("run.mode")}
          <select value={mode} disabled={active} onChange={(e) => setMode(e.target.value as Mode)}>
            <option value="run">{t("run.modes.run")}</option>
            <option value="smoke">{t("run.modes.smoke")}</option>
          </select>
        </label>
        {/* Подсказка вне <label>: клик по «?» внутри метки уводил бы фокус в поле. */}
        <div className="run-field">
          <span className="field-label">
            <label htmlFor={`${uid}-max`}>{t("run.maxInFlight")}</label> <Hint k="hints.run.maxInFlight" label={t("run.maxInFlight")} />
          </span>
          <NumberInput id={`${uid}-max`} optional aria-invalid={maxInvalid || undefined} min={1} step={1} value={maxInFlight} disabled={active} onChange={setMaxInFlight} />
        </div>
      </div>
      <div className="row">
        <button type="button" className="primary" disabled={!canStart} onClick={() => launch.mutate()}>
          {t("run.start")}
        </button>
        <button type="button" disabled={!(active || busy) || stopRun.isPending || run.status === "stopping"} onClick={() => stopRun.mutate()}>
          {t("run.stop")}
        </button>
        {blocked && (
          <button type="button" disabled={check.isPending} onClick={() => check.mutate()}>
            {t("run.recheck")}
          </button>
        )}
        <span className="muted" aria-live="polite">
          {t(`run.status.${run.status}`)}
        </span>
      </div>
      {busy && (
        <p className="warn" role="status">
          {t("run.busy")}
        </p>
      )}
      {blocked && <p className="warn">{t("run.checkFailed")}</p>}
      {findings && findings.length > 0 && <CheckReport findings={findings} onOpen={(file) => openFile.mutate(file)} />}
      {run.error && !busy && <ErrorBanner error={run.error} />}
      {launch.error && <ErrorBanner error={launch.error} />}
      {check.error && <ErrorBanner error={check.error} />}
      {stopRun.error && <ErrorBanner error={stopRun.error} />}
      {openFile.error && <ErrorBanner error={openFile.error} />}
      {suites.error && <ErrorBanner error={suites.error} onRetry={() => void suites.refetch()} />}
      {profiles.error && <ErrorBanner error={profiles.error} onRetry={() => void profiles.refetch()} />}
      {project.error && <ErrorBanner error={project.error} onRetry={() => void project.refetch()} />}
      {/* Смок всегда идёт внутри приложения (спека §4.6) — агентов стенда не опрашиваем. */}
      {mode === "smoke" ? <p className="muted">{t("run.smokeLocal")}</p> : selectedEnv && <AgentsStatus path={path} env={selectedEnv} live={run.agents} />}
    </section>
  );
}
