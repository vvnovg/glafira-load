import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { CHANGELOG_FILES, VERSION_FILES, cargoVersion, checkVersions, versionsOf } from "./check-version.mjs";

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

test("checkVersions: с тегом дата-заглушка xx в разделе списка изменений — проблема", () => {
  const ok = { "package.json": '{"version":"0.2.0"}' };
  const log = (date) => `# Changelog\n\n## 0.3.0\nReleased 2026-12-xx.\n- next\n\n## 0.2.0\nReleased ${date}. Stage L2.\n- a\n\n## 0.1.0\nReleased 2026-10-31.\n`;
  // заглушка в разделе тега
  const bad = checkVersions(ok, "v0.2.0", { "CHANGELOG.md": log("2026-11-xx"), "docs/changelog/ru.md": log("2026-11-02") });
  assert.equal(bad.length, 1, bad.join("\n"));
  assert.ok(bad[0].includes("CHANGELOG.md") && bad[0].includes("2026-11-xx"), bad[0]);
  // дата проставлена; заглушка в разделе другой версии не мешает
  assert.deepEqual(checkVersions(ok, "v0.2.0", { "CHANGELOG.md": log("2026-11-02") }), []);
  // без тега списки изменений не проверяются
  assert.deepEqual(checkVersions(ok, null, { "CHANGELOG.md": log("2026-11-xx") }), []);
  // раздела версии нет — это не про заглушку
  assert.deepEqual(checkVersions(ok, "v0.2.0", { "CHANGELOG.md": "# Changelog\n" }), []);
});

test("реальный репозиторий: все манифесты одной версии", () => {
  const files = Object.fromEntries(VERSION_FILES.map((p) => [p, readFileSync(p, "utf8")]));
  assert.deepEqual(checkVersions(files), []);
  assert.match(versionsOf(files).get("package.json"), /^\d+\.\d+\.\d+/);
  assert.ok(CHANGELOG_FILES.every((p) => readFileSync(p, "utf8").includes("## ")));
});
