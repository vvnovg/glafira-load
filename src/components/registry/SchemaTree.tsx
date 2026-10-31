import { useTranslation } from "react-i18next";
import type { SchemaSummary } from "../../api/types";

/** Сжатая схема деревом: имя, тип, обязательность, enum, пример, пометка обрезки. */
export function SchemaTree({ schema }: { schema: SchemaSummary }) {
  const { t } = useTranslation();
  return (
    <ul className="schema-tree">
      <li>
        <code>{schema.name}</code> <span className="muted">{schema.type}</span>
        {schema.required && <span className="warn" title={t("registry.required")}> *</span>}
        {schema.enum && schema.enum.length > 0 && <span className="muted"> [{schema.enum.join(" | ")}]</span>}
        {schema.example !== undefined && <span className="muted"> = {schema.example}</span>}
        {schema.truncated && <span className="muted"> {t("registry.truncated")}</span>}
        {schema.children && schema.children.length > 0 && (
          <ul>
            {schema.children.map((c, i) => (
              <li key={`${c.name}-${i}`}>
                <SchemaTree schema={c} />
              </li>
            ))}
          </ul>
        )}
      </li>
    </ul>
  );
}
