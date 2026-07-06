# gitwasm

**Repos that carry their own behavior.** Git hooks and merge drivers as
WebAssembly modules **committed into the repository itself**, executed in a
capability-scoped sandbox on every collaborator's machine — any OS, zero
install, safe to run even from a repo you just cloned from a stranger.

## Quickstart: end lockfile conflicts

Build or install the CLI first. From this source checkout:

```sh
rustup target add wasm32-wasip1 wasm32-unknown-unknown
cargo build --release -p gitwasm
```

That produces `./target/release/gitwasm`. Put that binary on PATH, or call it
by absolute path from the repository you want to protect.

Inside your repository, with `gitwasm` available on PATH:

```sh
gitwasm init lockfiles
git add .gitwasm .gitattributes
git commit -m "chore: add gitwasm lockfile merge drivers"
```

Collaborators run once per clone:

```sh
gitwasm install
```

From then on, supported generated files are merged structurally:
`package-lock.json`, `package.json`, `pnpm-lock.yaml`, `Cargo.lock`,
`yarn.lock` v1, `poetry.lock`, and `go.sum`.

To exercise the broader sandbox, hook, signing, and verdict audit story:

```sh
./demo/run-demo.sh                    # or demo\run-demo.ps1 on Windows
```

The demo exercises eight scenarios end-to-end in a throwaway repo: npm,
pnpm, Cargo, and `go.sum` merge drivers; a staged AWS key blocked by the
sandboxed pre-commit hook; opt-in commit-lint; signed `.gitwasm/` tamper
failure; and re-derivable verdict audit.

## The problem

Git is the world's most deployed database with no safe way to ship code that
governs it. All of git's extension points — hooks, merge drivers, filters —
require every collaborator to manually install platform-specific tooling.
Hooks can't be committed *by design*, because auto-running arbitrary code from
a clone would be a security disaster. So in practice nobody uses these
features, and we all suffer lockfile conflicts, unenforced conventions, and
leaked secrets.

## Why wasm dissolves this

1. **Trust** — a module is sandboxed: it sees one mounted directory of copies,
   its argv, stdout/stderr, and nothing else. No network, no env, no
   filesystem, plus fuel (CPU) and memory limits. Running committed code
   becomes safe *by construction*. The full honest threat model is in
   [SECURITY.md](SECURITY.md).
2. **Portability** — one `.wasm` blob runs identically on Windows, macOS,
   Linux, CI. The behavior is versioned with the code it governs: check out a
   2-year-old commit and you get the merge semantics it was written under.

## Layout

```
crates/gitwasm/            host CLI (wasmtime embed): init / install / list / sign / verify / hook / merge / run
modules/lockfile-merge/    structural 3-way JSON merge (package-lock.json, package.json)
modules/cargo-lock-merge/  structural 3-way merge for Cargo.lock
modules/yarn-lock-merge/   structural 3-way merge for yarn.lock v1
modules/poetry-lock-merge/ structural 3-way merge for poetry.lock
modules/pnpm-lock-merge/   structural 3-way merge for pnpm-lock.yaml
modules/lineset-merge/     set-algebra 3-way merge for line-set files (go.sum) — a WASI 0.2 component
modules/secret-scan/       pre-commit scanner over the staged-tree snapshot
modules/commit-lint/       conventional-commit linter (commit-msg hook, opt-in)
demo/                     end-to-end demos (sh + ps1), run in CI on all 3 OSes
SPEC.md                   the .gitwasm/ convention — written for second implementations
SECURITY.md               exact sandbox guarantees and non-guarantees
```

A consuming repo commits only `.gitwasm/` (manifest + wasm blobs) and
`.gitattributes` lines — see [SPEC.md](SPEC.md) for the format and the two
module ABIs: WASI **preview1** command modules (`wasm32-wasip1`, any language),
and typed **WASI 0.2 components** whose merge world imports nothing at all. The
host runs both; a repo can mix them freely.

## How execution works

`gitwasm hook <name>` materializes the *staged* tree (what is actually about
to be committed) into a temp dir and mounts it **read-only** as the module's
entire world; message hooks additionally get the message as `COMMIT_MSG`.
`gitwasm merge` mounts a temp dir containing exactly `base`/`ours`/`theirs`;
the module writes `result`; nonzero exit leaves a normal git conflict for the
human. A **component** merge driver skips the mount entirely: the host calls
its typed `merge3(base, ours, theirs, path) -> result<bytes, conflict>` with an
empty linker, so the module never sees a filesystem at all. Every run is fuel-,
memory-, and wall-clock-limited.

## Signing

`gitwasm keygen` once, `gitwasm sign` after changing `.gitwasm/`: every file
(including the hook shims) is hashed and ed25519-signed. Collaborators' clones
pin the signing key at `gitwasm install`; from then on tampered or unsigned
`.gitwasm/` content **refuses to execute**. Details in [SPEC.md](SPEC.md) §6
and [SECURITY.md](SECURITY.md).

## Verdicts

Every module run is a pure function of content-addressed inputs, so gitwasm
records each merge as a **verdict** — `hash(module) + hash(inputs) → {exit,
result}`, with the module and inputs stored content-addressed under
`.git/gitwasm/`.

Replay is a local cache optimization. `gitwasm audit` is the proof step: it
re-runs stored module bytes against stored input blobs and checks that the
recorded outcome reproduces. Future shared verdicts must remain unaudited until
the local host re-derives them or the user explicitly trusts their provenance.

## Roadmap

- More drivers: `Gemfile.lock`; tree-sitter semantic merge for source files.
- A typed **hook** world for components (merge drivers are typed as of the
  WASI 0.2 component ABI — see [SPEC.md](SPEC.md) §5.2; hooks are still
  preview1).
- **Verdict distribution**: merge runs are already recorded as content-addressed,
  re-derivable verdicts (`gitwasm verdicts` / `gitwasm audit`, SPEC.md §8) and
  memoized per clone. Making them travel through a git ref — so a team or CI
  can audit or explicitly trust shared provenance before reusing them — is the
  next step. Hooks join merges as verdict producers next.
- **Upstream**: the goal is not this tool — it is `.gitwasm/` as an open
  convention git hosts understand and, eventually, native sandboxed-module
  support in git itself. This repo is the reference implementation.

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at
your option. Contributions are welcome under the same terms — see
[CONTRIBUTING.md](CONTRIBUTING.md).
