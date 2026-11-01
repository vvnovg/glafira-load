import { useState } from "react";
import { useTranslation } from "react-i18next";
import type { OpKind, Scenario, Step, SuiteView } from "../../api/types";

/** Вид и операция шага: ключ шага — вариант `Action`. */
export function stepAction(step: Step): { kind: OpKind; operation: string } {
  if ("http" in step) return { kind: "http", operation: step.http.operation };
  if ("kafka_produce" in step) return { kind: "kafka_produce", operation: step.kafka_produce.operation };
  return { kind: "kafka_consume", operation: step.kafka_consume.operation };
}

/** Число проверок шага: `status`, каждый `jsonpath`, `schema`, `max_ms`; пустой `expect` serde не пишет вовсе. */
export function checksCount(step: Step): number {
  const e = step.expect;
  if (!e) return 0;
  return (e.status != null ? 1 : 0) + Object.keys(e.jsonpath ?? {}).length + (e.schema ? 1 : 0) + (e.max_ms != null ? 1 : 0);
}

/** Операция, которую ведёт сценарий: `drives` либо операция первого шага (как `Scenario::drives` в Rust). */
export function drivesOf(s: Scenario): string {
  return s.drives ?? (s.steps[0] ? stepAction(s.steps[0]).operation : "");
}

/** Id шага: заданный или `step<N>` (1-based), как `Scenario::step_ids`. */
export const stepId = (step: Step, index: number): string => step.id ?? `step${index + 1}`;

/** Отступ строки: число ведущих пробелов и табов. */
const indentOf = (line: string): number => line.length - line.trimStart().length;

/**
 * Строки текста с `from` по `to` (1-based, `to` не включается): блок шага от его строки. Блок кончается на
 * `to` (следующий ключ карты `lines`) либо раньше — на первой непустой строке с отступом не больше, чем у строки
 * шага (маркер `- `): так в блок не попадают `on_fail`, соседние сценарии и верхнеуровневые `data:`/`defaults:`.
 * Хвостовые пустые строки отбрасываются; `\r` (CRLF) убирается; `from` за концом текста — пусто.
 */
export function sliceYaml(text: string, from: number, to: number): string {
  const lines = text.split(/\r?\n/);
  const start = Math.min(lines.length, Math.max(1, from) - 1);
  let end = Math.min(lines.length, Math.max(start, to - 1));
  if (start < lines.length) {
    const indent = indentOf(lines[start]);
    for (let i = start + 1; i < end; i++) {
      if (lines[i].trim() !== "" && indentOf(lines[i]) <= indent) {
        end = i;
        break;
      }
    }
  }
  const out = lines.slice(start, end);
  while (out.length > 0 && out[out.length - 1].trim() === "") out.pop();
  return out.join("\n");
}

/** Ближайшая строка карты `lines` после `from`, иначе `fallback` (строка за концом текста). */
export function nextLine(lines: Record<string, number>, from: number, fallback: number): number {
  let best = fallback;
  for (const l of Object.values(lines)) if (l > from && l < best) best = l;
  return best;
}

/** Сценарии и шаги деревом; выбранный шаг показан фрагментом YAML только для чтения. */
export function SuiteTree({ view, onOpenFile }: { view: SuiteView; onOpenFile: () => void }) {
  const { t } = useTranslation();
  const [selected, setSelected] = useState<string | null>(null);
  const suite = view.suite;
  if (!suite) return <p className="muted">{t("suite.noSuite")}</p>;
  // Строка «за концом текста»: последний шаг режется до конца файла.
  const total = view.text.split(/\r?\n/).length + 1;
  const line = selected ? view.lines[selected] : undefined;
  const fragment = line ? sliceYaml(view.text, line, nextLine(view.lines, line, total)) : null;
  return (
    <div className="suite-body">
      <ul role="tree" className="tree suite-tree" aria-label={t("suite.scenarios")}>
        {suite.scenarios.map((s, i) => (
          <li role="treeitem" key={`${i}:${s.id}`} aria-expanded="true" aria-label={s.id}>
            <div className="tree-conn">
              <strong>{s.id}</strong>
              <span className="muted">{t("suite.drives", { operation: drivesOf(s) })}</span>
              <span className="kind-badge">{t(`suite.onFail.${s.on_fail}`)}</span>
              <span className="muted">{t("suite.steps", { n: s.steps.length })}</span>
            </div>
            <ul role="group" className="tree">
              {s.steps.map((step, j) => {
                const key = `scenarios[${i}].steps[${j}]`;
                const a = stepAction(step);
                return (
                  <li role="treeitem" key={key} aria-selected={selected === key}>
                    <button type="button" className={selected === key ? "link tree-name active" : "link tree-name"} onClick={() => setSelected(key)}>
                      <span className={`kind-badge kind-${a.kind}`}>{t(`registry.kinds.${a.kind}`)}</span> {stepId(step, j)} <code>{a.operation}</code>{" "}
                      <span className="muted">{t("suite.checks", { n: checksCount(step) })}</span>
                    </button>
                  </li>
                );
              })}
            </ul>
          </li>
        ))}
      </ul>
      <div className="suite-step">
        {selected && line === undefined ? (
          // Rust не нашёл строку шага: не путаем с «шаг не выбран».
          <p className="muted">{t("suite.lineUnknown")}</p>
        ) : fragment === null ? (
          <p className="muted">{t("suite.selectStep")}</p>
        ) : (
          <>
            <div className="row">
              <span className="muted">{t("suite.line", { line })}</span>
              <button type="button" onClick={onOpenFile}>
                {t("suite.openFile")}
              </button>
            </div>
            <pre data-testid="step-yaml">
              <code>{fragment}</code>
            </pre>
          </>
        )}
      </div>
    </div>
  );
}
