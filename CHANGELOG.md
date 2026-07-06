# Changelog

## Unreleased

### Added

- **Verdicts** — every merge run is now recorded as a content-addressed,
  re-derivable fact: `hash(module) + hash(inputs) → {exit_code, result_hash}`,
  with the module and all inputs stored content-addressed under `.git/gitwasm/`.
  An identical merge replays its recorded result instead of re-executing (replay
  is refused if the cached result no longer hashes to its address). New commands
  `gitwasm verdicts` (list) and `gitwasm audit` (re-derive every verdict and
  confirm it reproduces — nonzero exit if any fails). Kill-switch:
  `GITWASM_NO_VERDICTS=1`. This is the foundation of the roadmap's memoized,
  trustlessly shareable checks. (SPEC.md §8)
- **WASI 0.2 component module ABI** for merge drivers, coexisting with
  preview1. A component exports the typed `gitwasm:merge/driver` world
  (`wit/driver.wit`): `merge3(base, ours, theirs, path) -> result<bytes,
  conflict>`. Its world **imports nothing** — no filesystem, argv, env, clock,
  or stdio — so the host instantiates it with an empty linker and mounts no
  directory at all. That is a strictly stronger sandbox than preview1. The host
  detects the ABI from the wasm preamble and runs both kinds transparently.
  (SPEC.md §5.2)
- **`lineset-merge` is now the reference component**: built to
  `wasm32-unknown-unknown` and wrapped into a component with `wit_component` at
  build time; its pure merge logic is still unit-tested on the host.

### Fixed

- CLI no longer panics on a broken pipe (`gitwasm list | head`); it exits
  quietly on SIGPIPE like a normal Unix tool. (Closes the 0.3 known papercut.)

### Notes

- Building now also needs `rustup target add wasm32-unknown-unknown`.
- Maintainers: rebuild and `gitwasm sign` to refresh the committed
  `.gitwasm/lineset-merge.wasm` blob to the component build.

## 0.3.0 — 2026-07-05

### Added

- **`yarn-lock-merge` module** (yarn.lock v1, stock pattern): atomic-block
  3-way merge keyed by descriptor line; higher version wins on concurrent
  bumps; refuses yarn berry (v2+) files rather than corrupting them;
  parse→serialize round-trips byte-identically.
- **`poetry-lock-merge` module** (poetry.lock, stock pattern): `name@version`
  entry-set merge; merge-introduced duplicates collapse to the higher
  version; conflicting `metadata.content-hash` takes theirs with a
  `poetry lock --no-update` warning; marker-based multi-version package sets
  survive intact.

### Known papercuts

- `gitwasm <cmd> | head`-style early pipe closure makes the CLI panic on
  broken pipe instead of exiting quietly.

## 0.2.0 — 2026-07-05

The trust release.

### Added

- **Signed manifests**: `gitwasm keygen` / `sign` / `verify` / `trust`.
  ed25519 signatures over every file in `.gitwasm/` — including the hook
  shims git executes natively. `install` pins signers per clone (TOFU);
  every subsequent hook/merge run verifies **fail-closed**. (SPEC.md §6)
- **Wall-clock deadline** on module runs (`limits.wall_ms`, default 60s) —
  catches stalls that fuel metering can't.
- **Output sanitization**: module stdout/stderr is captured and stripped of
  control/escape bytes before reaching the terminal.
- **`lineset-merge` module**: set-algebra 3-way merge for line-set files;
  `go.sum` is a stock pattern.
- **`package.json` is now a stock merge pattern** for `lockfile-merge` —
  validated on a real repo where npm accepted the merged result with zero
  rewrites.
- `.gitwasm/** -text` gitattributes line, so EOL conversion can never break
  signature hashes across platforms.
- Demo scenario 5: tamper with a signed module blob → execution refused.

## 0.1.0 — 2026-07-05

Initial release: `gitwasm init/install/list/hook/merge/run`; sandboxed
(wasmtime) WASI modules committed in-repo; fuel + memory limits; stock
modules `lockfile-merge`, `cargo-lock-merge`, `secret-scan`, `commit-lint`;
SPEC/SECURITY/CONTRIBUTING; 3-OS CI with end-to-end demos.
