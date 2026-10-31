import { QueryClient, QueryClientProvider, useQuery } from "@tanstack/react-query";
import { act, render, renderHook } from "@testing-library/react";
import type { ReactNode } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { TabActiveContext, useRefetchInterval, useTabActive } from "./tabActive";

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

describe("useRefetchInterval", () => {
  const inTab = (active: boolean) => ({ children }: { children: ReactNode }) => <TabActiveContext.Provider value={active}>{children}</TabActiveContext.Provider>;

  it("показанная вкладка — выбранный интервал, 0 (выключено) — false", () => {
    expect(renderHook(() => useRefetchInterval(10_000)).result.current).toBe(10_000);
    expect(renderHook(() => useRefetchInterval(10_000), { wrapper: inTab(true) }).result.current).toBe(10_000);
    expect(renderHook(() => useRefetchInterval(0), { wrapper: inTab(true) }).result.current).toBe(false);
  });

  it("скрытая вкладка — false при любом интервале", () => {
    expect(renderHook(() => useRefetchInterval(10_000), { wrapper: inTab(false) }).result.current).toBe(false);
  });
});

describe("автообновление и скрытая вкладка", () => {
  afterEach(() => vi.useRealTimers());

  function Poller({ fetcher }: { fetcher: () => Promise<number> }) {
    const refetchInterval = useRefetchInterval(1000);
    useQuery({ queryKey: ["poll"], queryFn: fetcher, refetchInterval });
    return null;
  }

  it("запрос опрашивается, пока вкладка показана, и молчит, пока она скрыта", async () => {
    vi.useFakeTimers({ toFake: ["setInterval", "clearInterval"] });
    const fetcher = vi.fn().mockResolvedValue(1);
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    const ui = (active: boolean) => (
      <QueryClientProvider client={client}>
        <TabActiveContext.Provider value={active}>
          <Poller fetcher={fetcher} />
        </TabActiveContext.Provider>
      </QueryClientProvider>
    );
    // По секунде: ответ предыдущего запроса приходит между тиками, иначе тики схлопываются в один запрос.
    const advance = async (ms: number) => {
      for (let left = ms; left > 0; left -= 1000) {
        await act(async () => {
          vi.advanceTimersByTime(Math.min(left, 1000));
        });
      }
    };
    const view = render(ui(true));
    await act(async () => undefined); // ответ первого запроса
    expect(fetcher).toHaveBeenCalledTimes(1);
    await advance(3000);
    expect(fetcher).toHaveBeenCalledTimes(4);

    // Вкладку скрыли: запросов больше нет, сколько бы ни прошло времени.
    view.rerender(ui(false));
    await advance(60_000);
    expect(fetcher).toHaveBeenCalledTimes(4);

    // Вкладка снова показана: опрос продолжается с отсчёта заново.
    view.rerender(ui(true));
    await advance(999);
    expect(fetcher).toHaveBeenCalledTimes(4);
    await advance(1);
    expect(fetcher).toHaveBeenCalledTimes(5);
  });
});
