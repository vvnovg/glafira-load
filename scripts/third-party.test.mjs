import { test } from "node:test";
import assert from "node:assert/strict";
import { C_LIBRARIES, npmRows, render, rustRows } from "./third-party.mjs";

const pkg = (id, name, version, extra = {}) => ({ id, name, version, source: "registry+https://github.com/rust-lang/crates.io-index", license: "MIT", license_file: null, repository: `https://example.org/${name}`, ...extra });
const metadata = {
  packages: [
    pkg("app", "glafira-load-testing", "0.3.0", { source: null, license: "Apache-2.0" }),
    pkg("a", "alpha", "1.0.0"),
    pkg("b", "beta", "2.0.0", { license: "MIT OR Apache-2.0" }),
    pkg("c", "gamma", "0.1.0", { license: null, license_file: "LICENSE.txt" }),
    pkg("d", "devonly", "9.9.9"),
    pkg("e", "unused", "0.0.1"),
    pkg("f", "serde_json", "1.0.0"),
    pkg("g", "serde-untagged", "0.1.0"),
    pkg("h", "Inflector", "0.11.0"),
  ],
  resolve: {
    root: "app",
    nodes: [
      { id: "app", deps: [{ pkg: "a", dep_kinds: [{ kind: null }] }, { pkg: "b", dep_kinds: [{ kind: "build" }] }, { pkg: "d", dep_kinds: [{ kind: "dev" }] }, { pkg: "f", dep_kinds: [{ kind: null }] }, { pkg: "g", dep_kinds: [{ kind: null }] }, { pkg: "h", dep_kinds: [{ kind: null }] }] },
      { id: "a", deps: [{ pkg: "c", dep_kinds: [{ kind: null }] }] },
      { id: "b", deps: [] },
      { id: "c", deps: [] },
      { id: "d", deps: [{ pkg: "e", dep_kinds: [{ kind: null }] }] },
      { id: "e", deps: [] },
      { id: "f", deps: [] },
      { id: "g", deps: [] },
      { id: "h", deps: [] },
    ],
  },
};

test("rustRows: обычные и build-зависимости от корня, без dev, без локальных крейтов", () => {
  const rows = rustRows(metadata, ["glafira-load-testing"]);
  assert.deepEqual(
    rows.filter((r) => ["alpha", "beta", "gamma"].includes(r.name)).map((r) => `${r.name} ${r.version} ${r.license}`),
    ["alpha 1.0.0 MIT", "beta 2.0.0 MIT OR Apache-2.0", "gamma 0.1.0 see LICENSE.txt in the crate"],
  );
  assert.equal(rows.find((r) => r.name === "alpha").url, "https://example.org/alpha");
  assert.ok(!rows.some((r) => ["glafira-load-testing", "devonly", "unused"].includes(r.name)));
});

test("rustRows: порядок по кодовым точкам, а не по локали («-» раньше «_», заглавные раньше строчных)", () => {
  // localeCompare дал бы alpha, beta, gamma, Inflector, serde_json, serde-untagged.
  assert.deepEqual(
    rustRows(metadata, ["glafira-load-testing"]).map((r) => r.name),
    ["Inflector", "alpha", "beta", "gamma", "serde-untagged", "serde_json"],
  );
});

test("npmRows: группы по лицензии разворачиваются в строки по версиям", () => {
  const rows = npmRows({
    MIT: [{ name: "react", versions: ["19.1.0"], homepage: "https://react.dev" }, { name: "i18next", versions: ["25.0.0", "25.1.0"] }],
    "Apache-2.0": [{ name: "zustand", versions: ["5.0.0"] }],
  });
  assert.deepEqual(
    rows.map((r) => `${r.name}@${r.version}:${r.license}`),
    ["i18next@25.0.0:MIT", "i18next@25.1.0:MIT", "react@19.1.0:MIT", "zustand@5.0.0:Apache-2.0"],
  );
  assert.equal(rows[2].url, "https://react.dev");
  assert.equal(rows[0].url, "");
});

test("render: разделы, счётчики, C-библиотеки (если есть) и экранирование «|»", () => {
  const rust = [{ name: "a|b", version: "1.0.0", license: "MIT", url: "" }];
  const npm = [{ name: "react", version: "19.1.0", license: "MIT", url: "https://react.dev" }];
  // C-библиотеки, которые собирают крейты -sys движка (rdkafka): раздел есть по умолчанию.
  assert.deepEqual(C_LIBRARIES.map((l) => l.name), ["librdkafka", "lz4", "OpenSSL", "zlib", "zstd"]);
  for (const l of C_LIBRARIES) assert.ok(l.license && l.url.startsWith("https://") && l.note, l.name);
  assert.match(render({ rust, npm }), /## C libraries compiled into the binary/);
  assert.doesNotMatch(render({ rust, npm, cLibraries: [] }), /C libraries/);
  const lib = { name: "libx", license: "BSD-2-Clause", url: "https://x", note: "x-sys" };
  const withC = render({ rust, npm, cLibraries: [lib] });
  assert.match(withC, /## C libraries compiled into the binary/);
  assert.ok(withC.includes("| libx | BSD-2-Clause |"));
  const md = render({ rust, npm });
  assert.match(md, /^# Third-party software/);
  assert.match(md, /^Glafira for Load Testing is licensed/m);
  assert.match(md, /## Rust crates \(1\)/);
  assert.ok(md.includes("| a\\|b | 1.0.0 | MIT |"));
  assert.match(md, /## npm packages in the frontend \(1\)/);
  assert.ok(md.includes("| react | 19.1.0 | MIT | https://react.dev |"));
  assert.ok(md.endsWith("\n"));
});

test("rustRows: зависимости всех поставляемых пакетов (приложение и CLI), а не только корня", () => {
  const two = {
    packages: [
      ...metadata.packages,
      pkg("cli", "glafira-load-cli", "0.3.0", { source: null, license: "Apache-2.0" }),
      pkg("k", "rdkafka", "0.37.0"),
    ],
    resolve: {
      root: null,
      nodes: [...metadata.resolve.nodes, { id: "cli", deps: [{ pkg: "k", dep_kinds: [{ kind: null }] }, { pkg: "a", dep_kinds: [{ kind: null }] }] }, { id: "k", deps: [] }],
    },
    workspace_members: ["app", "cli"],
  };
  const names = rustRows(two).map((r) => r.name);
  assert.ok(names.includes("rdkafka") && names.includes("alpha"), names.join(","));
  assert.equal(names.filter((n) => n === "alpha").length, 1);
  assert.throws(() => rustRows(two, ["nope"]), /nope/);
});
