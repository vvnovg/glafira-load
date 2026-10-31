import { createContext, useContext } from "react";

/**
 * Показана ли вкладка рабочей области. `Layout` держит смонтированными все открытые вкладки и прячет неактивные, поэтому
 * без этого признака их watch, опросы и автообновление шли бы в фоне. Вне вкладки (дерево подключений, тесты) — `true`.
 */
export const TabActiveContext = createContext(true);

/** Показана ли вкладка, в которой стоит компонент; у скрытой вкладки кластер не опрашивается. */
export function useTabActive(): boolean {
  return useContext(TabActiveContext);
}

/**
 * `refetchInterval` запроса с автообновлением: выбранный интервал (0 — выключено), пока вкладка показана, и `false`,
 * пока она скрыта. Вернувшись на вкладку, запрос заново отсчитывает интервал; свежие данные — по «Обновить».
 */
export function useRefetchInterval(interval: number): number | false {
  const active = useTabActive();
  return active && interval > 0 ? interval : false;
}
