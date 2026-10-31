import { baseName, joinProjectPath, projectRelative } from "./paths";

it("joins relative paths and keeps absolute ones", () => {
  expect(joinProjectPath("/home/u/p", "../orders")).toBe("/home/u/p/../orders");
  expect(joinProjectPath("/home/u/p/", "/abs/x")).toBe("/abs/x");
  expect(joinProjectPath("C:\\work\\p", "svc")).toBe("C:\\work\\p\\svc");
  expect(joinProjectPath("C:\\work\\p", "D:\\other")).toBe("D:\\other");
  expect(baseName("/a/b/c/")).toBe("c");
});

it("stores paths inside the project as relative and outside ones as absolute", () => {
  expect(projectRelative("/home/u/p", "/home/u/p/svc/orders")).toBe("svc/orders");
  expect(projectRelative("/home/u/p/", "/home/u/p/svc")).toBe("svc");
  expect(projectRelative("/home/u/p", "/home/u/p")).toBe("/home/u/p");
  expect(projectRelative("/home/u/p", "/home/u/pp/x")).toBe("/home/u/pp/x");
  expect(projectRelative("/home/u/p", "/home/u/orders")).toBe("/home/u/orders");
  expect(projectRelative("C:\\work\\p", "C:\\work\\p\\svc\\a")).toBe("svc/a");
});
