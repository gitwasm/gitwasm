# Changelog

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
