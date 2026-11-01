import { vi } from "vitest";
import type { ControllerEvent } from "./types";

// Имена команд и аргументов должны совпадать с `#[tauri::command]` (Tauri переводит snake_case аргументов в camelCase).
const core = vi.hoisted(() => {
  class Channel<T> {
    onmessage: (m: T) => void = () => undefined;
  }
  return { invoke: vi.fn(async (_cmd: string, _args?: Record<string, unknown>): Promise<unknown> => undefined), Channel };
});
vi.mock("@tauri-apps/api/core", () => core);

const { api } = await import("./client");

describe("api L2", () => {
  beforeEach(() => core.invoke.mockClear());

  it("calls suite, agent and run commands with the Rust argument names", async () => {
    await api.suiteList("/p");
    await api.suiteGet("/p", "orders");
    await api.suiteCheck("/p", "orders");
    await api.suiteCreate("/p", "orders", ["op-1"]);
    await api.agentSetToken("/p", "test", "gen-1", null);
    await api.agentProbe("/p", "test");
    await api.runStop("/p");
    await api.runList("/p");
    await api.runGet("/p", "runs/2026-11-01T10-00-00");
    await api.confirmClose(true);
    await api.confirmClose(false);
    expect(core.invoke.mock.calls).toEqual([
      ["suite_list", { path: "/p" }],
      ["suite_get", { path: "/p", name: "orders" }],
      ["suite_check", { path: "/p", name: "orders" }],
      ["suite_create", { path: "/p", name: "orders", operationIds: ["op-1"] }],
      ["agent_set_token", { path: "/p", envId: "test", agentId: "gen-1", token: null }],
      ["agent_probe", { path: "/p", envId: "test" }],
      ["run_stop", { path: "/p" }],
      ["run_list", { path: "/p" }],
      ["run_get", { path: "/p", dir: "runs/2026-11-01T10-00-00" }],
      ["confirm_close", { stop: true }],
      ["confirm_close", { stop: false }],
    ]);
  });

  it("starts a run with a channel that forwards controller events", async () => {
    const events: ControllerEvent[] = [];
    await api.runStart("/p", "orders", "nominal", "test", "run", null, (e) => events.push(e));
    const [cmd, args] = core.invoke.mock.calls[0];
    expect(cmd).toBe("run_start");
    expect(args).toMatchObject({ path: "/p", suite: "orders", profile: "nominal", env: "test", mode: "run", maxInFlight: null });
    const channel = (args as { onEvent: InstanceType<typeof core.Channel<ControllerEvent>> }).onEvent;
    channel.onmessage({ type: "started", run_id: "r1", dir: "/p/runs/x" });
    expect(events).toEqual([{ type: "started", run_id: "r1", dir: "/p/runs/x" }]);
  });

  it("passes backend errors through as AppError", async () => {
    core.invoke.mockRejectedValueOnce({ code: "RUN_BUSY", key: "errors.run.busy", message: "A run is already in progress in this project", retryable: false });
    await expect(api.runList("/p")).rejects.toMatchObject({ code: "RUN_BUSY" });
  });
});
