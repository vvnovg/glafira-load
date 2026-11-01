import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";
import { emptyRun, useRunStore } from "../../state/run";
import { RunCloseDialog, STOP_WAIT_MS } from "./RunCloseDialog";

// vi.mock поднимается над импортами — объекты моков создаются там же через vi.hoisted.
const { api, events } = vi.hoisted(() => ({
  api: { runStop: vi.fn(async () => undefined), confirmClose: vi.fn(async () => undefined), runStart: vi.fn() },
  events: { handler: null as null | (() => void), unlisten: vi.fn() },
}));
vi.mock("../../api/client", () => ({ api, toAppError: (e: unknown) => e }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (name: string, h: () => void) => {
    if (name === "run-close-requested") events.handler = h;
    return events.unlisten;
  }),
}));

const finishRun = (path: string) => act(() => useRunStore.setState((s) => ({ byProject: { ...s.byProject, [path]: { ...s.byProject[path], status: "done" } } })));

describe("RunCloseDialog", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    events.handler = null;
    useRunStore.setState({ byProject: { "/p": { ...emptyRun(), status: "running" }, "/q": { ...emptyRun(), status: "done" } }, closeRequested: false });
  });

  it("opens on the window close request and unsubscribes on unmount", async () => {
    const { unmount } = render(<RunCloseDialog />);
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

  it("stops the running projects, waits for them to finish and confirms the close", async () => {
    useRunStore.setState({ closeRequested: true });
    render(<RunCloseDialog />);
    await userEvent.click(screen.getByRole("button", { name: "Остановить и закрыть" }));
    expect(api.runStop).toHaveBeenCalledWith("/p");
    expect(api.runStop).toHaveBeenCalledTimes(1);
    expect(await screen.findByText("Останавливаем прогон…")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Остановить и закрыть" })).toBeDisabled();
    expect(api.confirmClose).not.toHaveBeenCalled();
    finishRun("/p");
    await waitFor(() => expect(api.confirmClose).toHaveBeenCalledTimes(1));
  });

  it("closes anyway when the run does not finish in time", async () => {
    vi.useFakeTimers();
    try {
      useRunStore.setState({ closeRequested: true });
      render(<RunCloseDialog />);
      fireEvent.click(screen.getByRole("button", { name: "Остановить и закрыть" }));
      await act(() => vi.advanceTimersByTimeAsync(STOP_WAIT_MS - 1));
      expect(api.confirmClose).not.toHaveBeenCalled();
      await act(() => vi.advanceTimersByTimeAsync(1));
      expect(api.confirmClose).toHaveBeenCalledTimes(1);
    } finally {
      vi.useRealTimers();
    }
  });

  it("closes without stopping, or cancels", async () => {
    useRunStore.setState({ closeRequested: true });
    render(<RunCloseDialog />);
    await userEvent.click(screen.getByRole("button", { name: "Закрыть, агенты доработают" }));
    expect(api.runStop).not.toHaveBeenCalled();
    await waitFor(() => expect(api.confirmClose).toHaveBeenCalled());
    await userEvent.click(screen.getByRole("button", { name: "Отмена" }));
    expect(useRunStore.getState().closeRequested).toBe(false);
    expect(screen.queryByRole("alertdialog")).toBeNull();
  });

  it("does not close after Cancel while the run is stopping", async () => {
    useRunStore.setState({ closeRequested: true });
    render(<RunCloseDialog />);
    await userEvent.click(screen.getByRole("button", { name: "Остановить и закрыть" }));
    await userEvent.click(screen.getByRole("button", { name: "Отмена" }));
    finishRun("/p");
    await act(() => Promise.resolve());
    expect(api.confirmClose).not.toHaveBeenCalled();
  });

  it("shows the error when the stop fails and keeps the dialog", async () => {
    api.runStop.mockRejectedValueOnce({ code: "INTERNAL", message: "boom", retryable: false });
    useRunStore.setState({ closeRequested: true });
    render(<RunCloseDialog />);
    await userEvent.click(screen.getByRole("button", { name: "Остановить и закрыть" }));
    expect(await screen.findByText("boom")).toBeInTheDocument();
    expect(api.confirmClose).not.toHaveBeenCalled();
    expect(screen.getByRole("alertdialog")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Остановить и закрыть" })).toBeEnabled();
  });
});
