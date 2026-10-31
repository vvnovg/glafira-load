import { renderHook } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { TabActiveContext, useTabActive } from "./tabActive";

describe("useTabActive", () => {
  it("вне вкладки (тесты, дерево) — true", () => {
    const { result } = renderHook(() => useTabActive());
    expect(result.current).toBe(true);
  });

  it("берёт значение у вкладки", () => {
    const { result } = renderHook(() => useTabActive(), { wrapper: ({ children }) => <TabActiveContext.Provider value={false}>{children}</TabActiveContext.Provider> });
    expect(result.current).toBe(false);
  });
});
