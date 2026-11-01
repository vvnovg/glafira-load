import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { vi } from "vitest";
import type { Agent, AgentStatus } from "../../api/types";
import { renderWithClient } from "../../test/render";
import { AGENT_ID_RE, EnvironmentAgents, validAgentUrl, validAgentWeight } from "./EnvironmentAgents";

// vi.mock поднимается над импортами — объект мока создаётся там же через vi.hoisted.
const api = vi.hoisted(() => ({
  agentSetToken: vi.fn(async (_path: string, _env: string, _agent: string, _token: string | null) => undefined),
  agentProbe: vi.fn(
    async (_path: string, _env: string): Promise<AgentStatus[]> => [
      { id: "gen-1", url: "http://10.0.1.11:7420", state: "ready", info: { version: "0.2.0", os: "linux", arch: "x86_64", cpus: 8, busy: null }, message: null },
      { id: "gen-2", url: "http://10.0.1.12:7420", state: "no_token", info: null, message: null },
    ],
  ),
}));
vi.mock("../../api/client", () => ({ api, toAppError: (e: unknown) => e }));

const initial: Agent[] = [
  { id: "gen-1", url: "http://10.0.1.11:7420", weight: 1 },
  { id: "gen-2", url: "http://10.0.1.12:7420", weight: 2 },
];

function Harness({ onChange, saved = ["gen-1", "gen-2"] }: { onChange?: (a: Agent[]) => void; saved?: string[] }) {
  const [agents, setAgents] = useState<Agent[]>(initial);
  return (
    <EnvironmentAgents
      path="/p"
      envId="test"
      agents={agents}
      saved={saved}
      onChange={(a) => {
        setAgents(a);
        onChange?.(a);
      }}
    />
  );
}

describe("EnvironmentAgents", () => {
  beforeEach(() => vi.clearAllMocks());

  it("saves the token into the OS keychain without writing it anywhere else", async () => {
    const onChange = vi.fn();
    renderWithClient(<Harness onChange={onChange} />);
    const field = screen.getByLabelText("Токен агента gen-1");
    expect(field).toHaveAttribute("type", "password");
    await userEvent.type(field, "s3cret");
    await userEvent.click(screen.getAllByRole("button", { name: "Сохранить токен" })[0]);
    expect(api.agentSetToken).toHaveBeenCalledWith("/p", "test", "gen-1", "s3cret");
    await waitFor(() => expect(field).toHaveValue(""));
    expect(screen.getByText("Токен сохранён в хранилище ОС")).toBeInTheDocument();
    // Токен не попадает в список агентов (а значит, и в манифест).
    expect(onChange).not.toHaveBeenCalled();
    await userEvent.click(screen.getAllByRole("button", { name: "Удалить токен" })[1]);
    expect(api.agentSetToken).toHaveBeenLastCalledWith("/p", "test", "gen-2", null);
    expect(await screen.findByText("Токен удалён из хранилища ОС")).toBeInTheDocument();
  });

  it("probes the agents and shows badges, including the missing token", async () => {
    renderWithClient(<Harness />);
    expect(api.agentProbe).not.toHaveBeenCalled();
    await userEvent.click(screen.getByRole("button", { name: "Проверить" }));
    expect(api.agentProbe).toHaveBeenCalledWith("/p", "test");
    const badges = await screen.findAllByTestId("agent-state");
    expect(badges.map((b) => b.textContent)).toEqual(["готов", "нет токена"]);
    expect(badges[0]).toHaveClass("state-ok");
    expect(badges[1]).toHaveClass("state-warn");
    expect(screen.getByText("0.2.0 · linux/x86_64 · 8 ядер")).toBeInTheDocument();
    expect(screen.getByText("токен не задан")).toBeInTheDocument();
    expect(screen.getByText("токен задан")).toBeInTheDocument();
    // После сохранения токена статусы перечитываются.
    await userEvent.type(screen.getByLabelText("Токен агента gen-2"), "t");
    await userEvent.click(screen.getAllByRole("button", { name: "Сохранить токен" })[1]);
    await waitFor(() => expect(api.agentProbe).toHaveBeenCalledTimes(2));
  });

  it("shows the busy run and the reason of an unreachable agent", async () => {
    api.agentProbe.mockResolvedValueOnce([
      { id: "gen-1", url: "http://10.0.1.11:7420", state: "busy", info: { version: "0.2.0", os: "linux", arch: "x86_64", cpus: 1, busy: "r-42" }, message: "r-42" },
      { id: "gen-2", url: "http://10.0.1.12:7420", state: "unreachable", info: null, message: "AGENT_UNREACHABLE: connection refused" },
    ]);
    renderWithClient(<Harness />);
    await userEvent.click(screen.getByRole("button", { name: "Проверить" }));
    const badges = await screen.findAllByTestId("agent-state");
    expect(badges.map((b) => b.className)).toEqual(["state-badge state-warn", "state-badge state-err"]);
    expect(screen.getByText("прогон r-42")).toBeInTheDocument();
    expect(screen.getByText("AGENT_UNREACHABLE: connection refused")).toBeInTheDocument();
  });

  it("adds and removes agents and flags bad ids, urls and weights", async () => {
    const onChange = vi.fn();
    renderWithClient(<Harness onChange={onChange} />);
    await userEvent.click(screen.getByRole("button", { name: "Добавить агента" }));
    expect(onChange).toHaveBeenLastCalledWith([...initial, { id: "", url: "", weight: 1 }]);
    const ids = screen.getAllByLabelText("ID агента");
    expect(ids[2]).toHaveAttribute("aria-invalid", "true");
    await userEvent.type(ids[2], "gen-1");
    expect(ids[2]).toHaveAttribute("aria-invalid", "true");
    expect(screen.getAllByLabelText("Адрес агента")[2]).toHaveAttribute("aria-invalid", "true");
    await userEvent.type(screen.getAllByLabelText("Адрес агента")[2], "http://10.0.1.13:7420");
    expect(screen.getAllByLabelText("Адрес агента")[2]).not.toHaveAttribute("aria-invalid");
    await userEvent.clear(screen.getAllByLabelText("Вес")[2]);
    await userEvent.type(screen.getAllByLabelText("Вес")[2], "0");
    expect(screen.getAllByLabelText("Вес")[2]).toHaveAttribute("aria-invalid", "true");
    // Токен нельзя сохранить агенту с плохим или повторённым ID.
    expect(screen.getAllByLabelText(/Токен агента/)[2]).toBeDisabled();
    await userEvent.click(screen.getByRole("button", { name: "Удалить агента gen-2" }));
    expect(onChange).toHaveBeenLastCalledWith([initial[0], expect.objectContaining({ id: "gen-1" })]);
  });

  it("asks to save the manifest before a token of a new agent can be stored", async () => {
    renderWithClient(<Harness saved={["gen-1"]} />);
    expect(screen.getByLabelText("Токен агента gen-1")).toBeEnabled();
    expect(screen.getByLabelText("Токен агента gen-2")).toBeDisabled();
    expect(screen.getByText("сохраните манифест, чтобы задать токен")).toBeInTheDocument();
  });

  it("disables the check when the environment has no saved agents", () => {
    renderWithClient(<Harness saved={[]} />);
    expect(screen.getByRole("button", { name: "Проверить" })).toBeDisabled();
  });

  it("validates ids, urls and weights like the manifest", () => {
    expect(AGENT_ID_RE.test("gen-1")).toBe(true);
    expect(AGENT_ID_RE.test("Gen 1")).toBe(false);
    expect(validAgentUrl("https://a:7420")).toBe(true);
    expect(validAgentUrl("10.0.1.11:7420")).toBe(false);
    expect(validAgentWeight(1)).toBe(true);
    expect(validAgentWeight(0)).toBe(false);
    expect(validAgentWeight(1.5)).toBe(false);
    expect(validAgentWeight(Number.NaN)).toBe(false);
  });
});
