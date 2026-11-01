import { tabId, tabProject, useWorkspace } from "./workspace";

describe("workspace tabs", () => {
  beforeEach(() => useWorkspace.setState({ tabs: [], activeId: null }));

  it("ids include the project path and the service id", () => {
    expect(tabId({ kind: "project", path: "/p/a" })).toBe("project:/p/a");
    expect(tabId({ kind: "project", path: null })).toBe("project:new");
    expect(tabId({ kind: "service", path: "/p/a", serviceId: "orders" })).toBe("service:/p/a:orders");
    expect(tabId({ kind: "registry", path: "/p/a" })).toBe("registry:/p/a");
    expect(tabId({ kind: "prod-load", path: "/p/a" })).toBe("prod-load:/p/a");
    expect(tabId({ kind: "profile", path: "/p/a" })).toBe("profile:/p/a");
    expect(tabId({ kind: "suite", path: "/p/a", name: "orders" })).toBe("suite:/p/a:orders");
    expect(tabId({ kind: "run", path: "/p/a" })).toBe("run:/p/a");
    expect(tabProject({ kind: "suite", path: "/p/a", name: "orders" })).toBe("/p/a");
    expect(tabId({ kind: "help" })).toBe("help");
  });

  it("closeForProject closes every tab of the project and keeps the rest", () => {
    const { open, closeForProject } = useWorkspace.getState();
    open({ kind: "registry", path: "/p/a" });
    open({ kind: "profile", path: "/p/a" });
    open({ kind: "registry", path: "/p/b" });
    closeForProject("/p/a");
    const { tabs, activeId } = useWorkspace.getState();
    expect(tabs.map((t) => t.id)).toEqual(["registry:/p/b"]);
    expect(activeId).toBe("registry:/p/b");
    expect(tabProject({ kind: "about" })).toBeNull();
  });
});
