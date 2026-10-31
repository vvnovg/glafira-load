import { useTranslation } from "react-i18next";
import type { Phase } from "../../api/types";

const KINDS: Phase["kind"][] = ["ramp", "steady", "steps"];

/** Предел числа ступеней одной фазы — как `MAX_STEPS` в расчёте профиля. */
export const MAX_STEPS = 1000;

function blank(kind: Phase["kind"]): Phase {
  switch (kind) {
    case "ramp":
      return { kind, minutes: 10, to_pct: 100 };
    case "steady":
      return { kind, minutes: 30 };
    case "steps":
      return { kind, step_pct: 25, step_minutes: 10, max_pct: 200 };
  }
}

/** Поля фазы, не прошедшие ту же проверку, что и в расчёте профиля (бэкенд всё равно проверит при сохранении). */
export function invalidPhaseFields(p: Phase): string[] {
  const bad: string[] = [];
  const nonNegative = (key: string, v: number) => {
    if (!Number.isFinite(v) || v < 0) bad.push(key);
  };
  const finite = (key: string, v: number) => {
    if (!Number.isFinite(v)) bad.push(key);
  };
  switch (p.kind) {
    case "ramp":
      nonNegative("minutes", p.minutes);
      finite("to_pct", p.to_pct);
      break;
    case "steady":
      nonNegative("minutes", p.minutes);
      break;
    case "steps":
      if (!Number.isFinite(p.step_pct) || p.step_pct <= 0) bad.push("step_pct");
      nonNegative("step_minutes", p.step_minutes);
      finite("max_pct", p.max_pct);
      if (!bad.includes("step_pct") && !bad.includes("max_pct") && Math.floor((p.max_pct + 1e-9) / p.step_pct) > MAX_STEPS) {
        bad.push("step_pct", "max_pct");
      }
      break;
  }
  return bad;
}

/** Список фаз: вид, числовые поля по виду, перестановка и удаление. */
export function PhaseEditor({ phases, onChange }: { phases: Phase[]; onChange: (p: Phase[]) => void }) {
  const { t } = useTranslation();
  const set = (i: number, p: Phase) => onChange(phases.map((x, j) => (j === i ? p : x)));
  const move = (i: number, d: -1 | 1) => {
    const j = i + d;
    if (j < 0 || j >= phases.length) return;
    const next = [...phases];
    [next[i], next[j]] = [next[j], next[i]];
    onChange(next);
  };
  const num = (i: number, p: Phase, key: string, label: string) => (
    <label key={key}>
      {label}
      <input
        type="number"
        aria-label={`${label} ${i + 1}`}
        aria-invalid={invalidPhaseFields(p).includes(key) || undefined}
        step="any"
        value={(p as unknown as Record<string, number>)[key]}
        onChange={(e) => set(i, { ...p, [key]: Number(e.target.value) } as Phase)}
      />
    </label>
  );
  return (
    <ol className="steps">
      {phases.map((p, i) => (
        <li key={i} className="row">
          <select aria-label={`${t("profile.phaseKind")} ${i + 1}`} value={p.kind} onChange={(e) => set(i, blank(e.target.value as Phase["kind"]))}>
            {KINDS.map((k) => (
              <option key={k} value={k}>
                {t(`profile.phases.${k}`)}
              </option>
            ))}
          </select>
          {p.kind === "ramp" && [num(i, p, "minutes", t("profile.minutes")), num(i, p, "to_pct", t("profile.toPct"))]}
          {p.kind === "steady" && num(i, p, "minutes", t("profile.minutes"))}
          {p.kind === "steps" && [num(i, p, "step_pct", t("profile.stepPct")), num(i, p, "step_minutes", t("profile.stepMinutes")), num(i, p, "max_pct", t("profile.maxPct"))]}
          <button type="button" className="icon" aria-label={`${t("profile.moveUp")} ${i + 1}`} disabled={i === 0} onClick={() => move(i, -1)}>
            ↑
          </button>
          <button type="button" className="icon" aria-label={`${t("profile.moveDown")} ${i + 1}`} disabled={i === phases.length - 1} onClick={() => move(i, 1)}>
            ↓
          </button>
          <button type="button" className="icon" aria-label={`${t("common.delete")} ${i + 1}`} onClick={() => onChange(phases.filter((_, j) => j !== i))}>
            ×
          </button>
        </li>
      ))}
      <li>
        <button type="button" onClick={() => onChange([...phases, blank("steady")])}>
          {t("profile.addPhase")}
        </button>
      </li>
    </ol>
  );
}
