import type { TFunction } from "i18next";
import type { TemplateLabels } from "../../api/types";

/** Порядок колонок A–L шаблона (как в `glafira-load-xlsx`): id, сервис, тип, имя, описание, среднее, пик, ошибки, запрос, ответ, p95/лаг, комментарий. */
const COLUMN_KEYS = ["id", "service", "kind", "name", "summary", "avg", "peak", "errorPct", "reqBytes", "respBytes", "p95", "comment"] as const;
const HELP_KEYS = ["line1", "line2", "line3", "line4"] as const;
const PARAM_KEYS = ["stand", "periodFrom", "periodTo", "system", "filledBy"] as const;

/** Подписи листов и колонок на языке интерфейса (решение плана 5): 12 колонок, 12 примечаний, 5 параметров. */
export function templateLabels(t: TFunction): TemplateLabels {
  return {
    sheetOperations: t("template.sheets.operations"),
    sheetParams: t("template.sheets.params"),
    sheetHelp: t("template.sheets.help"),
    columns: COLUMN_KEYS.map((k) => t(`template.columns.${k}`)),
    notes: COLUMN_KEYS.map((k) => t(`template.notes.${k}`, { defaultValue: "" })),
    params: PARAM_KEYS.map((k) => t(`template.params.${k}`)),
    helpLines: HELP_KEYS.map((k) => t(`template.help.${k}`)),
  };
}
