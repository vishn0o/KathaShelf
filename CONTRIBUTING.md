# Contributing to KathaShelf

Thanks for helping improve KathaShelf. The project is intentionally small and
dependency-light, so keep changes focused and idiomatic.

## Development setup

```bash
git clone https://github.com/vishn0o/KathaShelf.git
cd KathaShelf
cargo run
```

You need a stable Rust toolchain and a C compiler for the bundled SQLite build.

## Before opening a pull request

Run the full quality gate:

```bash
cargo fmt
cargo check --all-targets
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release
```

All five must pass. The release profile uses `panic = "abort"` and symbol
stripping, so check that no `unwrap()`/`expect()` was introduced on a
user/network/file path.

## Ground rules

- Keep everything local-first — no account, telemetry, or cloud dependency.
- Preserve the Warm Paper / Cozy Night design tokens and the 240px sidebar.
- Schema changes must be additive and safe via `ALTER TABLE ... ADD COLUMN`.
- Network calls must run on background threads and use timeouts.
- Malayalam text must keep working with the bundled Manjari font.

## Commit style

Short imperative subject lines, e.g. `Fix cover cache eviction on delete`.

## Reporting bugs

Open an issue with your OS, Rust version, steps to reproduce, and whether the
problem happens offline. Include the app version from `Cargo.toml`.
