import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";
import type { ProdLoad, TemplateLabels, TemplatePreview } from "../../api/types";
import { renderWithClient } from "../../test/render";
import { ProdLoadPanel } from "./ProdLoadPanel";

// vi.mock поднимается над импортами — объекты моков создаются там же через vi.hoisted.
const { api, preview } = vi.hoisted(() => {
  const source = { stand: "prod", period_from: "", period_to: "", system: "", filled_by: "", imported_at: "", file: "" };
  const preview: TemplatePreview = {
    filled: ["http:o:POST /a"],
    empty: [],
    unknown_ids: ["http:o:GET /old"],
    invalid: [{ row: 3, id: "kafka_consume:t", column: "peak", value: "5", reason: "peak_below_avg" }],
    diff: [{ id: "http:o:POST /a", old: null, new: { avg: 1, peak: 2 } }],
    source,
    result: null,
    skipped_rows: 2,
  };
  const api = {
    projectView: vi.fn(async () => ({ path: "/p", manifest: { format: 1, name: "orders", services: [], environments: [] }, problems: [], readOnly: false, missingServices: [] })),
    prodLoadGet: vi.fn(async (): Promise<ProdLoad> => ({ format: 1, source: { ...source, stand: "" }, operations: { "http:o:POST /a": { avg: 1, peak: 2 } } })),
    registryList: vi.fn(async () => [
      { id: "http:o:POST /a", service: "o", kind: "http", name: "a", summary: "", source: "openapi", status: "active", lastVersion: "v", topicGuessed: false },
      { id: "kafka_consume:t", service: "o", kind: "kafka_consume", name: "t", summary: "", source: "schema", status: "active", lastVersion: "v", topicGuessed: false },
    ]),
    prodLoadExportTemplate: vi.fn(async (_path: string, _file: string, _labels: TemplateLabels, _prefill: boolean) => 2),
    prodLoadPreviewImport: vi.fn(async (): Promise<TemplatePreview> => preview),
    prodLoadApplyImport: vi.fn(async (_path: string, _file: string): Promise<ProdLoad> => ({ format: 1, source, operations: {} })),
  };
  return { api, preview };
});
vi.mock("../../api/client", () => ({ api, toAppError: (e: unknown) => e }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ save: vi.fn(async () => "/out/t.xlsx"), open: vi.fn(async () => "/in/filled.xlsx") }));

describe("ProdLoadPanel", () => {
  it("exports a template with labels and shows uncovered operations", async () => {
    renderWithClient(<ProdLoadPanel path="/p" />);
    expect(await screen.findByText("Операций без цифр ПРОМ: 1")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Выгрузить шаблон…" }));
    expect(await screen.findByText(/сохранён в \/out\/t.xlsx/)).toBeInTheDocument();
    const [, file, labels, prefill] = api.prodLoadExportTemplate.mock.calls[0];
    expect(file).toBe("/out/t.xlsx");
    expect(labels.columns).toHaveLength(12);
    expect(labels.notes).toHaveLength(12);
    expect(labels.params).toHaveLength(5);
    expect(prefill).toBe(true);
  });

  it("previews an import and blocks apply while there are invalid cells", async () => {
    renderWithClient(<ProdLoadPanel path="/p" />);
    await userEvent.click(await screen.findByRole("button", { name: "Импортировать…" }));
    expect(await screen.findByText(/Заполнено 1, пустых 0, неизвестных ID 1, с ошибками 1/)).toBeInTheDocument();
    expect(screen.getByText("пик меньше среднего")).toBeInTheDocument();
    expect(screen.getByText("Строк с цифрами без ID, пропущено: 2")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Применить" })).toBeDisabled();
  });

  it("applies a clean preview and closes it", async () => {
    const result: ProdLoad = { format: 1, source: preview.source, operations: { "http:o:POST /a": { avg: 1, peak: 2 } } };
    api.prodLoadPreviewImport.mockResolvedValueOnce({ ...preview, invalid: [], skipped_rows: 0, result });
    api.prodLoadApplyImport.mockResolvedValueOnce(result);
    renderWithClient(<ProdLoadPanel path="/p" />);
    await userEvent.click(await screen.findByRole("button", { name: "Импортировать…" }));
    const applyButton = await screen.findByRole("button", { name: "Применить" });
    expect(applyButton).toBeEnabled();
    await userEvent.click(applyButton);
    expect(api.prodLoadApplyImport).toHaveBeenCalledWith("/p", "/in/filled.xlsx");
    await waitFor(() => expect(screen.queryByText("Предпросмотр импорта")).not.toBeInTheDocument());
  });
});
