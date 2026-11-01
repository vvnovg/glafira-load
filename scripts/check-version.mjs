#!/usr/bin/env node
// Одна версия во всех манифестах: package.json, src-tauri/tauri.conf.json и каждый Cargo.toml с [package].
// `node scripts/check-version.mjs [vX.Y.Z]` — с тегом сверяет и его. Только модули node: (запускается в CI без зависимостей).
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

/** Версия из Cargo.toml: поле `version = "…"` внутри секции [package]; null — секции нет (workspace). */
export function cargoVersion(text) {
  const section = /^\[package\]\s*$([\s\S]*?)(?=^\[|(?![\s\S]))/m.exec(text);
  if (!section) return null;
  return /^version\s*=\s*"([^"]+)"/m.exec(section[1])?.[1] ?? null;
}

/** files: { путь: текст }. Версия каждого файла; Cargo.toml без [package] пропускается. */
export function versionsOf(files) {
  const out = new Map();
  for (const [path, text] of Object.entries(files)) {
    if (path.endsWith("Cargo.toml")) {
      const v = cargoVersion(text);
      if (v !== null) out.set(path, v);
    } else out.set(path, JSON.parse(text).version);
  }
  return out;
}

/** Проблемы: разные версии или тег не совпадает с версией. Пустой список — всё согласовано. */
export function checkVersions(files, tag = null) {
  const versions = versionsOf(files);
  const problems = [];
  const distinct = [...new Set(versions.values())];
  if (distinct.length > 1) {
    for (const [path, v] of versions) problems.push(`${path}: версия ${v}`);
    problems.push(`версии расходятся: ${distinct.join(", ")}`);
  }
  if (tag !== null) {
    const m = /^v(\d+\.\d+\.\d+(?:-[0-9A-Za-z.]+)?)$/.exec(tag);
    if (!m) problems.push(`тег ${tag} не вида vX.Y.Z`);
    else if (distinct.length === 1 && distinct[0] !== m[1]) problems.push(`тег ${tag}, а в манифестах версия ${distinct[0]}`);
  }
  return problems;
}

export const VERSION_FILES = [
  "package.json",
  "src-tauri/tauri.conf.json",
  "src-tauri/Cargo.toml",
  "crates/glafira-load-model/Cargo.toml",
  "crates/glafira-load-contracts/Cargo.toml",
  "crates/glafira-load-xlsx/Cargo.toml",
  "crates/glafira-load-engine/Cargo.toml",
  "crates/glafira-load-cli/Cargo.toml",
];

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const files = Object.fromEntries(VERSION_FILES.map((p) => [p, readFileSync(p, "utf8")]));
  // Пустой аргумент (`node scripts/check-version.mjs $TAG` без тега) — как его отсутствие.
  const problems = checkVersions(files, process.argv[2] || null);
  if (problems.length) {
    console.error(problems.map((p) => `✗ ${p}`).join("\n"));
    process.exit(1);
  }
  console.log(`✓ версия ${versionsOf(files).get("package.json")} во всех ${VERSION_FILES.length} манифестах`);
}
