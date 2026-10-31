import { keepPreviousData, useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../api/client";
import type { Basis, Coefficients, Override, Profile, ProfileFile } from "../../api/types";
import { useHelpChapter } from "../../state/helpContext";
import { tabId } from "../../state/workspace";
import { ErrorBanner } from "../common/ErrorBanner";
import { Hint } from "../common/Hint";
import { NumberInput } from "../common/NumberInput";
import { invalidPhaseFields, PhaseEditor } from "./PhaseEditor";
import { TimelineChart } from "./TimelineChart";

const EMPTY_ROWS: Record<string, Override> = {};

/** Коэффициент допустим, если он конечен и неотрицателен (так же проверяет бэкенд). */
const validK = (v: number | undefined): boolean => v === undefined || (Number.isFinite(v) && v >= 0);

/** Копия без structuredClone: файл профиля — чистый JSON. */
const clone = (f: ProfileFile): ProfileFile => JSON.parse(JSON.stringify(f)) as ProfileFile;

/** Есть ли в черновике значения, которые бэкенд отклонит при сохранении. */
function hasInvalid(f: ProfileFile): boolean {
  const c = f.coefficients;
  const rows = [...Object.values(c.overrides.services ?? EMPTY_ROWS), ...Object.values(c.overrides.operations ?? EMPTY_ROWS)];
  return (
    !validK(c.k_env) ||
    !validK(c.k_growth) ||
    rows.some((o) => !validK(o.k_env) || !validK(o.k_growth)) ||
    Object.values(f.profiles).some((p) => p.phases.some((ph) => invalidPhaseFields(ph).length > 0))
  );
}

function OverrideTable({ title, rows, onChange }: { title: string; rows: Record<string, Override>; onChange: (r: Record<string, Override>) => void }) {
  const { t } = useTranslation();
  const [key, setKey] = useState("");
  const setRow = (k: string, o: Override) => onChange({ ...rows, [k]: o });
  const field = (k: string, o: Override, which: "k_env" | "k_growth") => (
    <NumberInput
      optional
      aria-label={`${title}: ${which} ${k}`}
      aria-invalid={!validK(o[which]) || undefined}
      value={o[which]}
      onChange={(v) => setRow(k, { ...o, [which]: v })}
    />
  );
  return (
    <section>
      <h4>{title}</h4>
      <table>
        <tbody>
          {Object.entries(rows).map(([k, o]) => (
            <tr key={k}>
              <td>
                <code>{k}</code>
              </td>
              <td>{field(k, o, "k_env")}</td>
              <td>{field(k, o, "k_growth")}</td>
              <td>
                <button type="button" className="icon" aria-label={`${t("common.delete")} ${k}`} onClick={() => onChange(Object.fromEntries(Object.entries(rows).filter(([x]) => x !== k)))}>
                  ×
                </button>
              </td>
            </tr>
          ))}
          <tr>
            <td>
              <input aria-label={`${title}: ${t("profile.overrideKey")}`} placeholder={t("profile.overrideKey")} value={key} onChange={(e) => setKey(e.target.value)} />
            </td>
            <td colSpan={3}>
              <button
                type="button"
                disabled={!key.trim() || key.trim() in rows}
                onClick={() => {
                  setRow(key.trim(), {});
                  setKey("");
                }}
              >
                {t("profile.addOverride")}
              </button>
            </td>
          </tr>
        </tbody>
      </table>
    </section>
  );
}

export function ProfilePanel({ path }: { path: string }) {
  const { t } = useTranslation();
  const qc = useQueryClient();
  useHelpChapter(tabId({ kind: "profile", path }), "profile");
  const project = useQuery({ queryKey: ["project", path], queryFn: () => api.projectView(path) });
  const file = useQuery({ queryKey: ["profile", path], queryFn: () => api.profileGet(path) });
  const [draft, setDraft] = useState<ProfileFile | null>(null);
  const [name, setName] = useState("nominal");
  const [newName, setNewName] = useState("");
  useEffect(() => {
    if (file.data) {
      setDraft(clone(file.data));
      if (!(name in file.data.profiles)) setName(Object.keys(file.data.profiles)[0] ?? "");
    }
  }, [file.data]); // eslint-disable-line react-hooks/exhaustive-deps
  const save = useMutation({
    mutationFn: (p: ProfileFile) => api.profileSave(path, p),
    // Новый файл входит в ключ расчёта — пересчёт запустится сам, без отдельной инвалидации.
    onSuccess: (p) => qc.setQueryData(["profile", path], p),
  });
  const computed = useQuery({
    queryKey: ["profileComputed", path, name, file.data],
    queryFn: () => api.profileCompute(path, name),
    enabled: !!file.data && !!name && name in (file.data?.profiles ?? {}),
    // Цели и график не пропадают на время пересчёта после сохранения или смены профиля.
    placeholderData: keepPreviousData,
  });

  if (file.error) return <ErrorBanner error={file.error} onRetry={() => file.refetch()} />;
  if (!draft || !file.data) return <p className="empty">{t("common.loading")}</p>;
  const dirty = JSON.stringify(draft) !== JSON.stringify(file.data);
  const invalid = hasInvalid(draft);
  const profile: Profile | undefined = draft.profiles[name];
  const setProfile = (patch: Partial<Profile>) => profile && setDraft({ ...draft, profiles: { ...draft.profiles, [name]: { ...profile, ...patch } } });
  const setCoef = (patch: Partial<Coefficients>) => setDraft({ ...draft, coefficients: { ...draft.coefficients, ...patch } });
  const overrides = draft.coefficients.overrides;
  const trimmed = newName.trim();
  const taken = trimmed in draft.profiles;
  // Профиль, которого ещё нет в сохранённом файле, не посчитать: сначала сохранить.
  const savedProfile = !!profile && name in file.data.profiles;
  const c = savedProfile ? computed.data : undefined;
  // Пока манифест не прочитан или в нём есть проблемы, профиль только просматривается.
  const readOnly = !project.data || project.data.readOnly;

  return (
    <div className="page">
      <div className="toolbar">
        <h2>
          {t("profile.title")} <Hint k="hints.profile.title" label={t("profile.title")} />
        </h2>
        <button type="button" disabled={!dirty || invalid || save.isPending || readOnly} onClick={() => save.mutate(draft)}>
          {t("common.save")}
        </button>
        <button type="button" disabled={!dirty || readOnly} onClick={() => setDraft(clone(file.data))}>
          {t("project.revert")}
        </button>
        {project.data?.readOnly && <span className="muted">{t("common.readOnly")}</span>}
        {dirty && <span className="muted">{t("profile.unsaved")}</span>}
        {invalid && <span className="warn">{t("profile.invalidValues")}</span>}
      </div>
      {project.error && <ErrorBanner error={project.error} onRetry={() => project.refetch()} />}
      {save.error && <ErrorBanner error={save.error} />}
      <fieldset className="plain" disabled={readOnly}>
        <section className="form">
          <h3>
            {t("profile.coefficients")} <Hint k="hints.profile.coefficients" label={t("profile.coefficients")} />
          </h3>
          <div className="row">
            <label>
              k_env
              <NumberInput aria-label="k_env" aria-invalid={!validK(draft.coefficients.k_env) || undefined} value={draft.coefficients.k_env} onChange={(v) => setCoef({ k_env: v ?? Number.NaN })} />
            </label>
            <label>
              k_growth
              <NumberInput aria-label="k_growth" aria-invalid={!validK(draft.coefficients.k_growth) || undefined} value={draft.coefficients.k_growth} onChange={(v) => setCoef({ k_growth: v ?? Number.NaN })} />
            </label>
          </div>
          <OverrideTable title={t("profile.serviceOverrides")} rows={overrides.services ?? EMPTY_ROWS} onChange={(services) => setCoef({ overrides: { ...overrides, services } })} />
          <OverrideTable title={t("profile.operationOverrides")} rows={overrides.operations ?? EMPTY_ROWS} onChange={(operations) => setCoef({ overrides: { ...overrides, operations } })} />
        </section>
      </fieldset>
      <section className="form">
        <h3>{t("profile.profiles")}</h3>
        <div className="row">
          <select aria-label={t("profile.profiles")} value={name} onChange={(e) => setName(e.target.value)}>
            {Object.keys(draft.profiles).map((n) => (
              <option key={n}>{n}</option>
            ))}
          </select>
          <input aria-label={t("profile.newName")} placeholder={t("profile.newName")} value={newName} disabled={readOnly} onChange={(e) => setNewName(e.target.value)} />
          <button
            type="button"
            disabled={!trimmed || taken || readOnly}
            onClick={() => {
              setDraft({ ...draft, profiles: { ...draft.profiles, [trimmed]: { basis: "avg", phases: [{ kind: "ramp", minutes: 10, to_pct: 100 }, { kind: "steady", minutes: 30 }] } } });
              setName(trimmed);
              setNewName("");
            }}
          >
            {t("profile.addProfile")}
          </button>
          <button
            type="button"
            disabled={!trimmed || taken || !profile || readOnly}
            onClick={() => {
              // Порядок профилей сохраняем: переименованный остаётся на своём месте.
              setDraft({ ...draft, profiles: Object.fromEntries(Object.entries(draft.profiles).map(([n, p]) => [n === name ? trimmed : n, p])) });
              setName(trimmed);
              setNewName("");
            }}
          >
            {t("profile.renameProfile")}
          </button>
          <button
            type="button"
            disabled={Object.keys(draft.profiles).length <= 1 || !profile || readOnly}
            onClick={() => {
              const rest = Object.fromEntries(Object.entries(draft.profiles).filter(([n]) => n !== name));
              setDraft({ ...draft, profiles: rest });
              setName(Object.keys(rest)[0] ?? "");
            }}
          >
            {t("profile.removeProfile")}
          </button>
        </div>
        {profile && (
          <fieldset className="plain" disabled={readOnly}>
            <label>
              {t("profile.basis")}
              <select aria-label={t("profile.basis")} value={profile.basis} onChange={(e) => setProfile({ basis: e.target.value as Basis })}>
                <option value="avg">{t("profile.basisAvg")}</option>
                <option value="peak">{t("profile.basisPeak")}</option>
              </select>
            </label>
            <PhaseEditor phases={profile.phases} onChange={(phases) => setProfile({ phases })} />
          </fieldset>
        )}
      </section>
      {profile && !savedProfile && <p className="muted">{t("profile.saveToCompute")}</p>}
      {savedProfile && computed.error && <ErrorBanner error={computed.error} onRetry={() => computed.refetch()} />}
      {c && (
        <section className={computed.isPlaceholderData ? "stale" : undefined} aria-busy={computed.isPlaceholderData || undefined}>
          <h3>{t("profile.targets", { profile: c.profile, total: c.total.toFixed(1), minutes: c.total_minutes })}</h3>
          {dirty && <p className="muted">{t("profile.computedFromSaved")}</p>}
          <TimelineChart points={c.timeline} total={c.total} />
          <table>
            <thead>
              <tr>
                <th>ID</th>
                <th>{t("registry.service")}</th>
                <th>{t("profile.basisValue")}</th>
                <th>k_env</th>
                <th>k_growth</th>
                <th>{t("profile.target")}</th>
                <th>{t("profile.share")}</th>
              </tr>
            </thead>
            <tbody>
              {c.targets.map((x) => (
                <tr key={x.id}>
                  <td>
                    <code>{x.id}</code>
                  </td>
                  <td>{x.service}</td>
                  <td>{x.basis_value}</td>
                  <td>{x.k_env}</td>
                  <td>{x.k_growth}</td>
                  <td>{x.target.toFixed(2)}</td>
                  <td>{(x.share * 100).toFixed(1)}%</td>
                </tr>
              ))}
            </tbody>
          </table>
          {c.uncovered.length > 0 && (
            <details>
              <summary className="warn">{t("profile.uncovered", { count: c.uncovered.length })}</summary>
              <ul>
                {c.uncovered.map((id) => (
                  <li key={id}>
                    <code>{id}</code>
                  </li>
                ))}
              </ul>
            </details>
          )}
        </section>
      )}
    </div>
  );
}
