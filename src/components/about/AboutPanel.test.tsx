import { act, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { renderWithClient } from "../../test/render";

vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: vi.fn() }));
vi.mock("../../api/client", async (importOriginal) => {
  const orig = await importOriginal<typeof import("../../api/client")>();
  return { ...orig, api: { getAppInfo: vi.fn(), getPolicyStatus: vi.fn(), checkUpdate: vi.fn(), installUpdate: vi.fn() } };
});

import { openUrl } from "@tauri-apps/plugin-opener";
import { api } from "../../api/client";
import { i18n } from "../../i18n";
import { LICENSE_URL, SUPPORT_URL } from "../../lib/links";
import { AboutPanel } from "./AboutPanel";

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(api.getAppInfo).mockResolvedValue({
    name: "Glafira for Load Testing", version: "0.1.0", commit: "abc1234", os: "macos", arch: "aarch64",
    dataDir: "/data", logDir: "/logs",
  });
  vi.mocked(api.getPolicyStatus).mockResolvedValue({
    managed: true, source: "/etc/x", updatesEnabled: true, updateUrl: null, language: null,
    errors: [{ key: "errors.policy.invalidBool", params: { key: "updatesEnabled", value: "2" }, text: "x" }],
  });
});

describe("AboutPanel", () => {
  it("версия, сборка, политика, ссылки и товарные знаки", async () => {
    const user = userEvent.setup();
    renderWithClient(<AboutPanel />);
    expect(await screen.findByText("Glafira for Load Testing")).toBeInTheDocument();
    expect(screen.getByText("0.1.0")).toBeInTheDocument();
    expect(screen.getByText("abc1234")).toBeInTheDocument();
    expect(await screen.findByText("задана")).toBeInTheDocument();
    expect(screen.getByText("Политика организации: updatesEnabled: недопустимое логическое 2")).toBeInTheDocument();
    expect(screen.getByText(/Apache Kafka — товарный знак/)).toBeInTheDocument();
    expect(screen.getByText("/data")).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Агент нагрузки" })).toBeInTheDocument();
    expect(screen.getByText(/glafira-load-macos-aarch64 -o \/usr\/local\/bin\/glafira-load/)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Apache 2.0" }));
    expect(openUrl).toHaveBeenCalledWith(LICENSE_URL);
    await user.click(screen.getByRole("button", { name: "Поддержать проект" }));
    expect(openUrl).toHaveBeenCalledWith(SUPPORT_URL);
  });

  it("ошибка — баннер с повтором", async () => {
    vi.mocked(api.getAppInfo).mockRejectedValueOnce({ code: "INTERNAL", key: "errors.internal", message: "x", retryable: false });
    const user = userEvent.setup();
    renderWithClient(<AboutPanel />);
    await user.click(await screen.findByRole("button", { name: "Повторить" }));
    expect(await screen.findByText("Glafira for Load Testing")).toBeInTheDocument();
  });

  it("обновление доступно: установка без подтверждения", async () => {
    vi.mocked(api.checkUpdate).mockResolvedValue({ status: "available", version: "0.2.0", notes: "Исправления", notesI18n: {}, date: null });
    vi.mocked(api.installUpdate).mockRejectedValueOnce({ code: "INTERNAL", message: "Не удалось установить обновление", retryable: false });
    const user = userEvent.setup();
    renderWithClient(<AboutPanel />);
    await user.click(await screen.findByRole("button", { name: "Проверить обновления" }));
    expect(await screen.findByText("Доступна версия 0.2.0")).toBeInTheDocument();
    expect(await screen.findByText("Исправления")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Установить и перезапустить" }));
    expect(api.installUpdate).toHaveBeenCalledTimes(1);
    expect(await screen.findByText("Не удалось установить обновление")).toBeInTheDocument();
  });

  it("заметки к обновлению — Markdown на языке интерфейса, ссылки — в системный браузер", async () => {
    vi.mocked(api.checkUpdate).mockResolvedValue({
      status: "available",
      version: "0.2.0",
      notes: "- **Fixes**",
      notesI18n: { en: "- **Fixes**", ru: "- **Исправления** для `registry`\n- [Подробнее](https://example.com/r)" },
      date: null,
    });
    const user = userEvent.setup();
    renderWithClient(<AboutPanel />);
    await user.click(await screen.findByRole("button", { name: "Проверить обновления" }));
    const notes = await screen.findByRole("region", { name: "Что нового в версии 0.2.0" });
    expect((await screen.findByText("Исправления")).tagName).toBe("STRONG");
    expect(screen.getByText("registry").tagName).toBe("CODE");
    expect(notes.querySelectorAll("li")).toHaveLength(2);
    expect(notes.textContent).not.toContain("**");
    expect(screen.queryByText(/Fixes/)).not.toBeInTheDocument();
    await user.click(screen.getByRole("link", { name: "Подробнее" }));
    expect(openUrl).toHaveBeenCalledWith("https://example.com/r");
  });

  it("смена языка меняет заметки без новой проверки; нет перевода — английские", async () => {
    vi.mocked(api.checkUpdate).mockResolvedValue({ status: "available", version: "0.2.0", notes: "- Fixes", notesI18n: { en: "- Fixes" }, date: null });
    const user = userEvent.setup();
    renderWithClient(<AboutPanel />);
    await user.click(await screen.findByRole("button", { name: "Проверить обновления" }));
    // Раздела на русском нет — английские заметки.
    expect(await screen.findByText("Fixes")).toBeInTheDocument();
    vi.mocked(api.checkUpdate).mockResolvedValue({ status: "available", version: "0.2.0", notes: "- Fixes", notesI18n: { en: "- Fixes", ru: "- Исправления" }, date: null });
    await user.click(screen.getByRole("button", { name: "Проверить обновления" }));
    expect(await screen.findByText("Исправления")).toBeInTheDocument();
    try {
      await act(() => i18n.changeLanguage("en"));
      expect(await screen.findByText("Fixes")).toBeInTheDocument();
      expect(api.checkUpdate).toHaveBeenCalledTimes(2);
    } finally {
      await act(() => i18n.changeLanguage("ru"));
    }
  });

  it("политика выключила обновления", async () => {
    vi.mocked(api.checkUpdate).mockResolvedValue({
      status: "disabled",
      reason: { key: "errors.updates.managed", params: {}, text: "Updates are managed by your organization" },
    });
    const user = userEvent.setup();
    renderWithClient(<AboutPanel />);
    await user.click(await screen.findByRole("button", { name: "Проверить обновления" }));
    expect(await screen.findByText("Обновления управляются вашей организацией")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Установить и перезапустить" })).not.toBeInTheDocument();
  });

  it("последняя версия; ошибка сети — баннер с повтором", async () => {
    vi.mocked(api.checkUpdate)
      .mockRejectedValueOnce({ code: "CONNECTION_FAILED", message: "Не удалось проверить обновления", retryable: true })
      .mockResolvedValueOnce({ status: "upToDate" });
    const user = userEvent.setup();
    renderWithClient(<AboutPanel />);
    await user.click(await screen.findByRole("button", { name: "Проверить обновления" }));
    expect(await screen.findByText("Не удалось проверить обновления")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Повторить" }));
    expect(await screen.findByText("Установлена последняя версия")).toBeInTheDocument();
  });
});
