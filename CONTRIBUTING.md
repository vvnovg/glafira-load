# Contributing

Thank you for your interest in Glafira for Load Testing.

## Before you start

- Open an issue first for anything bigger than a small fix, so that the approach can be agreed on.
- The working language of the repository is Russian: comments, commit messages and `docs/` are written in Russian.
  Issues and pull requests in English are welcome.

## Building and testing

See `README.md` («Требования», «Запуск», «Тесты»). Before opening a pull request run:

```bash
pnpm test && pnpm test:scripts && pnpm i18n:check && pnpm build
cargo fmt --manifest-path src-tauri/Cargo.toml --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml --lib
cargo deny --manifest-path src-tauri/Cargo.toml --config deny.toml check licenses
```

`cargo-deny` can be installed with `cargo install cargo-deny --locked --version 0.20.2`.

## Interface strings and the guide

Every new string goes to all dictionaries in `src/locales/` at once (see `docs/i18n/README.md` and
`docs/i18n/glossary.md`); `pnpm i18n:check` enforces it. Changes to the user guide go to every language in
`docs/guide/`.

## Shared code with Glafira for Kubernetes

Files listed in `PORTED.md` were copied from Glafira for Kubernetes (which in turn took much of them from Glafira for
Apache Kafka). A fix in such a file is usually needed in all of these projects: mention it in the pull request.

## Developer Certificate of Origin

Contributions are accepted under the Apache License 2.0 (`LICENSE`). Commits in pull requests from contributors
must be signed off (`git commit -s`), which certifies the
[Developer Certificate of Origin](https://developercertificate.org/). Pull requests whose commits lack
a `Signed-off-by` line are not merged.
