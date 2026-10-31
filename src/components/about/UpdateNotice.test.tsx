import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useWorkspace } from "../../state/workspace";
import { renderWithClient } from "../../test/render";

vi.mock("../../api/client", async (importOriginal) => {
  const orig = await importOriginal<typeof import("../../api/client")>();
  return { ...orig, api: { checkUpdate: vi.fn() } };
});

import { api } from "../../api/client";
import { UpdateNotice } from "./UpdateNotice";

describe("UpdateNotice", () => {
  beforeEach(() => {
    vi.mocked(api.checkUpdate).mockReset();
    useWorkspace.setState({ tabs: [], activeId: null });
  });

  it("показывает доступную версию и открывает «О программе»", async () => {
    vi.mocked(api.checkUpdate).mockResolvedValue({ status: "available", version: "0.3.0", notes: null, notesI18n: {}, date: null });
    const user = userEvent.setup();
    renderWithClient(<UpdateNotice />);
    expect(await screen.findByText("Доступна версия 0.3.0.")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Подробнее" }));
    expect(useWorkspace.getState().tabs.some((t) => t.kind === "about")).toBe(true);
    await user.click(screen.getByRole("button", { name: "Скрыть уведомление" }));
    expect(screen.queryByText("Доступна версия 0.3.0.")).not.toBeInTheDocument();
  });

  it("ошибка проверки ничего не показывает", async () => {
    vi.mocked(api.checkUpdate).mockRejectedValue({ code: "INVALID_INPUT", message: "Обновления не настроены в этой сборке", retryable: false });
    const { container } = renderWithClient(<UpdateNotice />);
    await vi.waitFor(() => expect(api.checkUpdate).toHaveBeenCalled());
    expect(container).toBeEmptyDOMElement();
  });
});
