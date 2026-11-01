import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";
import type { Agent, Project } from "../../api/types";
import { emptyRun, type StartParams, useRunStore } from "../../state/run";
import { RunCloseDialog } from "./RunCloseDialog";

// vi.mock поднимается над импортами — объекты моков создаются там же через vi.hoisted.
const { api, events } = vi.hoisted(() => ({
  api: { runStop: vi.fn(async () => undefined), confirmClose: vi.fn(async (_stop: boolean) => undefined), runStart: vi.fn() },
  events: { handler: null as null | (() => void), unlisten: vi.fn() },
}));
vi.mock("../../api/client", () => ({ api, toAppError: (e: unknown) => e }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (name: string, h: () => void) => {
    if (name === "run-close-requested") events.handler = h;
    return events.unlisten;
  }),
}));

const params = (env: string, mode: "run" | "smoke" = "run"): StartParams => ({ suite: "orders", profile: "nominal", env, mode, maxInFlight: null, timeline: [], total: 0 });
const agent: Agent = { id: "gen-1", url: "http://gen-1:7700", weight: 1 };
/** Манифест проекта в кэше запросов: стенд `remote` с агентом, `local` — без агентов. */
const project = (path: string): Project => ({
  path,
  manifest: { format: 1, name: "P", services: [], environments: [{ id: "remote", kind: "test", agents: [agent] }, { id: "local", kind: "test" }] },
  problems: [],
  readOnly: false,
  missingServices: [],
});

function renderDialog() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  client.setQueryData(["project", "/p"], project("/p"));
  client.setQueryData(["project", "/r"], project("/r"));
  return render(
    <QueryClientProvider client={client}>
      <RunCloseDialog />
    </QueryClientProvider>,
  );
}

const running = (p: StartParams) => ({ ...emptyRun(), status: "running" as const, params: p });

describe("RunCloseDialog", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    events.handler = null;
    useRunStore.setState({ byProject: { "/p": running(params("remote")), "/q": { ...emptyRun(), status: "done", params: params("local") } }, closeRequested: false });
  });

  it("opens on the window close request and unsubscribes on unmount", async () => {
    const { unmount } = renderDialog();
    expect(screen.queryByRole("alertdialog")).toBeNull();
    await waitFor(() => expect(events.handler).not.toBeNull());
    act(() => events.handler?.());
    expect(screen.getByRole("alertdialog", { name: "Прогон идёт" })).toHaveTextContent("агенты доработают сами");
    // Фокус уходит в диалог, Esc — отмена.
    expect(screen.getByRole("alertdialog")).toHaveFocus();
    await userEvent.keyboard("{Escape}");
    expect(screen.queryByRole("alertdialog")).toBeNull();
    unmount();
    await waitFor(() => expect(events.unlisten).toHaveBeenCalled());
  });

  it("asks the backend to stop the runs and close, and cannot be cancelled after that", async () => {
    // Ответа нет: бэкенд ждёт остановки и завершает приложение.
    api.confirmClose.mockImplementationOnce(() => new Promise<undefined>(() => undefined));
    useRunStore.setState({ closeRequested: true });
    renderDialog();
    await userEvent.click(screen.getByRole("button", { name: "Остановить и закрыть" }));
    expect(api.confirmClose).toHaveBeenCalledExactlyOnceWith(true);
    // Остановку делает `confirm_close`, а не `run_stop` по проектам.
    expect(api.runStop).not.toHaveBeenCalled();
    expect(screen.getByRole("status")).toHaveTextContent("Останавливаем прогон, приложение закроется");
    for (const name of ["Остановить и закрыть", "Закрыть, агенты доработают", "Отмена"]) expect(screen.getByRole("button", { name })).toBeDisabled();
    await userEvent.keyboard("{Escape}");
    expect(screen.getByRole("alertdialog")).toBeInTheDocument();
    expect(useRunStore.getState().closeRequested).toBe(true);
  });

  it("closes without stopping when the agents finish on their own", async () => {
    useRunStore.setState({ closeRequested: true });
    renderDialog();
    await userEvent.click(screen.getByRole("button", { name: "Закрыть, агенты доработают" }));
    expect(api.confirmClose).toHaveBeenCalledExactlyOnceWith(false);
    expect(api.runStop).not.toHaveBeenCalled();
  });

  it("says the run is interrupted when every active run executes inside the application", async () => {
    // Стенд без агентов и смок — встроенный агент; завершённый прогон `/q` не в счёт.
    useRunStore.setState({ byProject: { "/p": running(params("local")), "/r": running(params("remote", "smoke")), "/q": { ...emptyRun(), status: "done", params: params("remote") } }, closeRequested: true });
    renderDialog();
    expect(screen.getByRole("alertdialog")).toHaveTextContent("выполняется внутри приложения");
    expect(screen.queryByRole("button", { name: "Закрыть, агенты доработают" })).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: "Закрыть, прогон прервётся" }));
    expect(api.confirmClose).toHaveBeenCalledExactlyOnceWith(false);
  });

  it("keeps the agents wording when at least one run is on the stand's agents or the stand is unknown", () => {
    useRunStore.setState({ byProject: { "/p": running(params("local")), "/r": running(params("remote")) }, closeRequested: true });
    const { unmount } = renderDialog();
    expect(screen.getByRole("button", { name: "Закрыть, агенты доработают" })).toBeInTheDocument();
    unmount();
    // Манифеста проекта нет в кэше — не утверждаем, что прогон прервётся.
    useRunStore.setState({ byProject: { "/x": running(params("local")) } });
    renderDialog();
    expect(screen.getByRole("button", { name: "Закрыть, агенты доработают" })).toBeInTheDocument();
  });

  it("cancels without closing", async () => {
    useRunStore.setState({ closeRequested: true });
    renderDialog();
    await userEvent.click(screen.getByRole("button", { name: "Отмена" }));
    expect(useRunStore.getState().closeRequested).toBe(false);
    expect(screen.queryByRole("alertdialog")).toBeNull();
    expect(api.confirmClose).not.toHaveBeenCalled();
  });

  it("shows the error when the close fails and keeps the dialog", async () => {
    api.confirmClose.mockRejectedValueOnce({ code: "INTERNAL", message: "boom", retryable: false });
    useRunStore.setState({ closeRequested: true });
    renderDialog();
    await userEvent.click(screen.getByRole("button", { name: "Остановить и закрыть" }));
    expect(await screen.findByText("boom")).toBeInTheDocument();
    expect(screen.getByRole("alertdialog")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Остановить и закрыть" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "Отмена" })).toBeEnabled();
  });
});
