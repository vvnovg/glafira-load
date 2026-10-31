import { render, screen } from "@testing-library/react";
import { SchemaTree } from "./SchemaTree";

it("renders nested fields with markers", () => {
  render(<SchemaTree schema={{ name: "body", type: "object", children: [{ name: "id", type: "string", required: true, example: "A-1" }, { name: "status", type: "string", enum: ["NEW", "PAID"] }, { name: "deep", type: "object", truncated: true }] }} />);
  expect(screen.getByText("id").parentElement).toHaveTextContent("id string * = A-1");
  expect(screen.getByText("status").parentElement).toHaveTextContent("[NEW | PAID]");
  expect(screen.getByText("deep").parentElement).toHaveTextContent("обрезано");
});
