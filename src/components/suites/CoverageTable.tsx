import { useTranslation } from "react-i18next";
import type { Coverage, CoverageRow } from "../../api/types";
import { formatNumber } from "../../lib/format";
import { Hint } from "../common/Hint";

/** Расхождение заметно, если |delta| больше 10 % цели (при нулевой цели — любое ненулевое). */
export const isOff = (row: CoverageRow): boolean => Math.abs(row.delta) > 0.1 * row.target;

const signed = (v: number): string => (v > 0 ? `+${formatNumber(v, 1)}` : formatNumber(v, 1));

/** Покрытие профиля сьютом: операция, вид, цель, расчётная нагрузка, разница; «не покрыто» списком. */
export function CoverageTable({ coverage, profile }: { coverage: Coverage; profile: string }) {
  const { t } = useTranslation();
  return (
    <section>
      <h3>
        {t("suite.coverageTitle", { profile })} <Hint k="hints.suite.coverage" label={t("suite.coverageTitle", { profile })} />
      </h3>
      <table>
        <thead>
          <tr>
            <th>{t("suite.column.operation")}</th>
            <th>{t("suite.column.kind")}</th>
            <th>{t("suite.column.target")}</th>
            <th>{t("suite.column.planned")}</th>
            <th>{t("suite.column.delta")}</th>
          </tr>
        </thead>
        <tbody>
          {coverage.rows.map((r) => (
            <tr key={r.id}>
              <td>
                <code>{r.id}</code>
              </td>
              <td>
                <span className={`kind-badge kind-${r.kind}`}>{t(`registry.kinds.${r.kind}`)}</span>
              </td>
              <td>{formatNumber(r.target, 1)}</td>
              <td>{formatNumber(r.planned, 1)}</td>
              <td data-testid="delta" className={isOff(r) ? "warn" : undefined}>
                {signed(r.delta)}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
      {coverage.uncovered.length > 0 && (
        <details open>
          <summary className="warn">{t("suite.uncovered", { n: coverage.uncovered.length })}</summary>
          <ul>
            {coverage.uncovered.map((id) => (
              <li key={id}>
                <code>{id}</code>
              </li>
            ))}
          </ul>
        </details>
      )}
    </section>
  );
}
