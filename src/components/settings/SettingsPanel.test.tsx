import { act, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { LanguageSettings } from "../../api/types";
import { i18n } from "../../i18n";
import { renderWithClient } from "../../test/render";
import { useHelpContext } from "../../state/helpContext";

const base: LanguageSettings = {
  system: "ru-RU", systemMatch: "ru", chosen: null, policy: null, effective: "ru", reason: "system",
  languages: ["en", "ru"],
};

vi.mock("../../api/client", async (importOriginal) => {
  const orig = await importOriginal<typeof import("../../api/client")>();
  return { ...orig, api: { getLanguageSettings: vi.fn(), setLanguage: vi.fn() } };
});

import { api } from "../../api/client";
import { SettingsPanel } from "./SettingsPanel";

beforeEach(() => {
  vi.clearAllMocks();
});

// Выбор «en» переключает глобальный язык; сброс — в act, пока компоненты смонтированы.
afterEach(() => act(() => i18n.changeLanguage("ru")));

describe("SettingsPanel", () => {
  it("показывает язык системы и меняет язык", async () => {
    vi.mocked(api.getLanguageSettings).mockResolvedValue(base);
    vi.mocked(api.setLanguage).mockResolvedValue({ ...base, chosen: "en", effective: "en", reason: "user" });
    const user = userEvent.setup();
    renderWithClient(<SettingsPanel />);
    const select = await screen.findByLabelText("Язык интерфейса");
    expect(screen.getByRole("option", { name: "Как в системе (Русский)" })).toBeInTheDocument();
    expect(screen.getAllByRole("option").map((o) => o.textContent)).toEqual(expect.arrayContaining(["English", "Русский"]));
    await user.selectOptions(select, "en");
    expect(api.setLanguage).toHaveBeenCalledWith("en");
    expect(await screen.findByLabelText("Interface language")).toBeInTheDocument();
  });

  it("язык из политики нельзя сменить", async () => {
    vi.mocked(api.getLanguageSettings).mockResolvedValue({ ...base, policy: "ru", reason: "policy" });
    renderWithClient(<SettingsPanel />);
    expect(await screen.findByLabelText("Язык интерфейса")).toBeDisabled();
    expect(screen.getByText("Язык задан вашей организацией")).toBeInTheDocument();
  });

  it("F1 в настройках — глава «Начало работы»", async () => {
    vi.mocked(api.getLanguageSettings).mockResolvedValue(base);
    renderWithClient(<SettingsPanel />);
    await screen.findByLabelText("Язык интерфейса");
    expect(useHelpContext.getState().byTab.settings).toBe("getting-started");
  });
});
