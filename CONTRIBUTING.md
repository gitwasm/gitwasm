# Contributing to gitwasm

Thank you — module contributions are the whole point of this project. The
long-term goal is `.gitwasm/` as an open convention (see SPEC.md), and every
new module or second implementation strengthens it.

## Building

```
rustup target add wasm32-wasip1 wasm32-unknown-unknown
cargo build --release -p gitwasm     # build.rs compiles + embeds stock modules
cargo test --workspace
./demo/run-demo.sh                   # or demo\run-demo.ps1 on Windows
```

(`wasm32-unknown-unknown` is only needed for the component modules, which are
compiled there and then wrapped into WASI 0.2 components at build time.)

## Writing a module

Modules come in two flavors (SPEC.md §5). Read SPEC.md §4 for the exact
contracts; `modules/commit-lint` is the smallest preview1 example (~80 lines,
zero dependencies) and `modules/lineset-merge` the smallest component one.

- **preview1 command module** (`wasm32-wasip1`, any language): a hook scans
  the mounted staged tree and exits nonzero to block; a merge driver reads
  `base`/`ours`/`theirs` and writes `result`.
- **component merge module** (`wasm32-unknown-unknown` + `wit-bindgen`): export
  the typed `gitwasm:merge/driver` world from `wit/driver.wit` and return
  `ok(bytes)` or `err(reason)`. The world imports nothing, so keep the guest
  glue behind `#[cfg(target_arch = "wasm32")]` and put the real logic in a
  plain function you can unit-test on the host.

Keep modules deterministic (no clocks, no randomness) and dependency-light —
the blob ships inside user repositories, so size is a feature.

## Ground rules

- Conventional commit messages (`feat:`, `fix:`, ...). Yes, the repo
  dogfoods its own commit-lint module.
- New behavior needs a test or a demo scenario that proves it end-to-end.
- Security-relevant changes (anything touching `runner.rs` or the sandbox
  contract) get extra scrutiny; explain the capability impact in the PR.

## License

Dual MIT/Apache-2.0. By contributing you agree your work is licensed the
same way.
