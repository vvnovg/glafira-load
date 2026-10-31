import { createContext, useContext } from "react";

/**
 * Показана ли вкладка рабочей области. `Layout` держит смонтированными все открытые вкладки и прячет неактивные;
 * по этому признаку компонент скрытой вкладки может не делать фоновую работу. Вне вкладки (дерево проектов, тесты) — `true`.
 */
export const TabActiveContext = createContext(true);

/** Показана ли вкладка, в которой стоит компонент. */
export function useTabActive(): boolean {
  return useContext(TabActiveContext);
}
