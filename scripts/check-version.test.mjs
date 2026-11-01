import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { VERSION_FILES, cargoVersion, checkVersions, versionsOf } from "./check-version.mjs";

const cargo = (v) => `[package]\nname = "x"\nversion = "${v}"\nedition = "2024"\n\n[dependencies]\nserde = "1"\n`;
const workspace = `[workspace]\nresolver = "2"\nmembers = ["a"]\n\n[workspace.package]\nedition = "2024"\n`;

test("cargoVersion: версия из [package], workspace без [package] — null", () => {
  assert.equal(cargoVersion(cargo("0.2.0")), "0.2.0");
  assert.equal(cargoVersion(workspace), null);
  // version в другой секции не считается
  assert.equal(cargoVersion(`[package]\nname = "x"\n\n[dependencies]\nfoo = { version = "9.9.9" }\n`), null);
});

test("versionsOf и checkVersions: согласованные версии, расхождение, тег", () => {
  const ok = { "package.json": '{"version":"0.2.0"}', "src-tauri/tauri.conf.json": '{"version":"0.2.0"}', "a/Cargo.toml": cargo("0.2.0"), "Cargo.toml": workspace };
  assert.deepEqual([...versionsOf(ok)], [["package.json", "0.2.0"], ["src-tauri/tauri.conf.json", "0.2.0"], ["a/Cargo.toml", "0.2.0"]]);
  assert.deepEqual(checkVersions(ok), []);
  assert.deepEqual(checkVersions(ok, "v0.2.0"), []);
  const bad = checkVersions({ ...ok, "a/Cargo.toml": cargo("0.1.0") });
  assert.ok(bad.some((p) => p.includes("a/Cargo.toml") && p.includes("0.1.0")), bad.join("\n"));
  assert.ok(bad.some((p) => p.includes("расходятся")), bad.join("\n"));
  assert.ok(checkVersions(ok, "v0.3.0").some((p) => p.includes("v0.3.0") && p.includes("0.2.0")));
  assert.ok(checkVersions(ok, "0.2.0").some((p) => p.includes("не вида")));
  assert.deepEqual(checkVersions(ok, "v0.2.0-beta.1").length, 1);
});

test("реальный репозиторий: все манифесты одной версии", () => {
  const files = Object.fromEntries(VERSION_FILES.map((p) => [p, readFileSync(p, "utf8")]));
  assert.deepEqual(checkVersions(files), []);
  assert.match(versionsOf(files).get("package.json"), /^\d+\.\d+\.\d+/);
});
