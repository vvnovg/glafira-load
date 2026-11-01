import { screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { vi } from "vitest";
import { renderWithClient } from "../../test/render";
import { useWorkspace } from "../../state/workspace";
import { CreateSuiteDialog, SUITE_NAME_RE } from "./CreateSuiteDialog";

// vi.mock поднимается над импортами — объект мока создаётся там же через vi.hoisted.
const api = vi.hoisted(() => ({
  registryList: vi.fn(async () => [
    { id: "http:orders:POST /v1/orders", service: "orders", kind: "http", name: "createOrder", summary: "", source: "openapi", status: "active", lastVersion: "v2", topicGuessed: false },
    { id: "kafka_consume:orders.created", service: "orders", kind: "kafka_consume", name: "orders.created", summary: "", source: "asyncapi", status: "active", lastVersion: "v2", topicGuessed: false },
  ]),
  suiteCreate: vi.fn(async (_p: string, name: string) => ({ name, file: `suites/${name}/suite.yaml`, scenarios: 1 })),
}));
vi.mock("../../api/client", () => ({ api, toAppError: (e: unknown) => e }));

describe("CreateSuiteDialog", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useWorkspace.setState({ tabs: [], activeId: null });
  });

  it("validates the name, needs at least one operation, refreshes the suites node and opens the new suite tab", async () => {
    const onClose = vi.fn();
    const { client } = renderWithClient(<CreateSuiteDialog path="/p" onClose={onClose} />);
    const invalidate = vi.spyOn(client, "invalidateQueries");
    const dialog = screen.getByRole("dialog", { name: "Новый сьют" });
    expect(api.registryList).toHaveBeenCalledWith("/p", { status: "active" });
    const name = within(dialog).getByLabelText("Имя сьюта");
    await userEvent.type(name, "Bad Name");
    expect(name).toHaveAttribute("aria-invalid", "true");
    expect(within(dialog).getByText("a–z, 0–9 и дефис, до 64 символов")).toBeInTheDocument();
    expect(within(dialog).getByRole("button", { name: "Создать" })).toBeDisabled();
    await userEvent.clear(name);
    await userEvent.type(name, "orders-2");
    expect(name).not.toHaveAttribute("aria-invalid");
    expect(within(dialog).getByRole("button", { name: "Создать" })).toBeDisabled();
    await userEvent.click(await within(dialog).findByRole("checkbox", { name: /POST \/v1\/orders/ }));
    expect(within(dialog).getByText("Выбрано: 1")).toBeInTheDocument();
    await userEvent.click(within(dialog).getByRole("button", { name: "Создать" }));
    expect(api.suiteCreate).toHaveBeenCalledWith("/p", "orders-2", ["http:orders:POST /v1/orders"]);
    expect(invalidate).toHaveBeenCalledWith({ queryKey: ["suites", "/p"] });
    expect(useWorkspace.getState().activeId).toBe("suite:/p:orders-2");
    expect(onClose).toHaveBeenCalled();
  });

  it("filters operations by search and keeps the selection; Escape closes", async () => {
    const onClose = vi.fn();
    renderWithClient(<CreateSuiteDialog path="/p" onClose={onClose} />);
    await userEvent.click(await screen.findByRole("checkbox", { name: /orders\.created/ }));
    await userEvent.type(screen.getByLabelText("Поиск операций"), "post");
    expect(screen.queryByRole("checkbox", { name: /orders\.created/ })).toBeNull();
    expect(screen.getByRole("checkbox", { name: /POST \/v1\/orders/ })).toBeInTheDocument();
    expect(screen.getByText("Выбрано: 1")).toBeInTheDocument();
    await userEvent.keyboard("{Escape}");
    expect(onClose).toHaveBeenCalled();
  });

  it("works from the keyboard: focus starts in the dialog, Space ticks an operation, Tab stays inside", async () => {
    renderWithClient(<CreateSuiteDialog path="/p" onClose={() => undefined} />);
    const dialog = screen.getByRole("dialog", { name: "Новый сьют" });
    expect(dialog).toHaveFocus();
    await userEvent.tab();
    expect(screen.getByLabelText("Имя сьюта")).toHaveFocus();
    await userEvent.keyboard("orders-3");
    const box = await screen.findByRole("checkbox", { name: /POST \/v1\/orders/ });
    box.focus();
    await userEvent.keyboard(" ");
    expect(box).toBeChecked();
    // Последний элемент — «Отмена»; следующий Tab возвращается к первому полю диалога.
    screen.getByRole("button", { name: "Отмена" }).focus();
    await userEvent.tab();
    expect(screen.getByLabelText("Имя сьюта")).toHaveFocus();
  });

  it("returns focus to the opener on Escape and on Cancel", async () => {
    function Host() {
      const [open, setOpen] = useState(false);
      return (
        <>
          <button type="button" onClick={() => setOpen(true)}>
            Открыть диалог
          </button>
          {open && <CreateSuiteDialog path="/p" onClose={() => setOpen(false)} />}
        </>
      );
    }
    renderWithClient(<Host />);
    const opener = screen.getByRole("button", { name: "Открыть диалог" });
    await userEvent.click(opener);
    expect(screen.getByRole("dialog", { name: "Новый сьют" })).toHaveFocus();
    await userEvent.keyboard("{Escape}");
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(opener).toHaveFocus();
    await userEvent.click(opener);
    await userEvent.click(screen.getByRole("button", { name: "Отмена" }));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(opener).toHaveFocus();
  });

  it("explains an empty registry", async () => {
    api.registryList.mockImplementationOnce(async () => []);
    renderWithClient(<CreateSuiteDialog path="/p" onClose={() => undefined} />);
    expect(await screen.findByText("В реестре нет активных операций: сначала разберите сервис")).toBeInTheDocument();
  });

  it("accepts the same names as Rust", () => {
    expect(SUITE_NAME_RE.test("orders-2")).toBe(true);
    expect(SUITE_NAME_RE.test("a".repeat(64))).toBe(true);
    expect(SUITE_NAME_RE.test("a".repeat(65))).toBe(false);
    expect(SUITE_NAME_RE.test("Orders")).toBe(false);
    expect(SUITE_NAME_RE.test("")).toBe(false);
  });
});
