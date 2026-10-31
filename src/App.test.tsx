import { render, screen } from "@testing-library/react";
import { vi } from "vitest";

vi.mock("./api/client", async (importOriginal) => {
  const orig = await importOriginal<typeof import("./api/client")>();
  return {
    ...orig,
    api: {
      getLanguageSettings: vi.fn(() => new Promise(() => {})),
      checkUpdate: vi.fn().mockResolvedValue({ status: "upToDate" }),
      projectRecent: vi.fn().mockResolvedValue([]),
    },
  };
});

import App from "./App";

test("первый запуск: пустое дерево с подсказкой и пустая рабочая область", async () => {
  render(<App />);
  expect(await screen.findByText("Откройте папку проекта НТ или создайте новый.")).toBeInTheDocument();
  expect(screen.getByText("Откройте проект слева или создайте новый.")).toBeInTheDocument();
});
