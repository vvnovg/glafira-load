# Contributing

Thank you for your interest in Glafira for Load Testing.

## Before you start

- Open an issue first for anything bigger than a small fix, so that the approach can be agreed on.
- The working language of the repository is Russian: comments, commit messages and `docs/` are written in Russian.
  Issues and pull requests in English are welcome.

## Building and testing

The repository is a Cargo workspace: the app in `src-tauri/` and the crates `crates/glafira-load-{model,contracts,xlsx}`
(they do not depend on Tauri). See `README.md` ("Development") for the requirements. Before opening a pull request
run from the repository root:

```bash
pnpm install --frozen-lockfile
pnpm test && pnpm test:scripts && pnpm i18n:check && pnpm build
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo deny --manifest-path src-tauri/Cargo.toml --config deny.toml check licenses advisories
```

`cargo test --workspace` needs `git` in `PATH` (the version of a service clone is read with `git describe`).
`cargo-deny` can be installed with `cargo install cargo-deny --locked --version 0.20.2`. The same commands run in CI
(`.github/workflows/ci.yml`).

Contract importers are tested on the fixtures in `dev/fixtures/` (one per importer and per limitation); a new
importer rule comes with a fixture and a test next to the importer in `crates/glafira-load-contracts/src/`.

When a dependency is added or updated, regenerate the license list with `node scripts/third-party.mjs` and commit
`THIRD_PARTY.md`.

## Interface strings and the guide

Every new string goes to all dictionaries in `src/locales/` (`en.json`, `ru.json`) at once; `pnpm i18n:check`
enforces it. Strings shown by the Rust side carry a key and an English fallback (`ltext!`, `app_err!`); the check makes
sure the key exists in the dictionaries. Changes to the user guide go to every language in
`docs/guide/`; interface elements are referenced as `[[ui:key]]`, and the check makes sure every key exists.

## Shared code with the Glafira family

Much of the application shell (settings, policy, i18n, About, Help, the scripts in `scripts/`) was copied from
Glafira for Apache Kafka with the same file and function names, so that it can later move into shared code.
A fix in such a file is usually needed in the other Glafira apps too: mention it in the pull request.

## Developer Certificate of Origin

Contributions are accepted under the Apache License 2.0 (`LICENSE`). Commits in pull requests from contributors
must be signed off (`git commit -s`), which certifies the
[Developer Certificate of Origin](https://developercertificate.org/). Pull requests whose commits lack
a `Signed-off-by` line are not merged.
