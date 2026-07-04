# gitwasm

**Repos that carry their own behavior.** Git hooks and merge drivers as
WebAssembly modules **committed into the repository itself**, executed in a
capability-scoped sandbox on every collaborator's machine — any OS, zero
install, safe to run even from a repo you just cloned from a stranger.

```
$ gitwasm init
gitwasm: [on ] lockfile-merge.wasm    structural 3-way merge for JSON lockfiles
gitwasm: [on ] cargo-lock-merge.wasm  structural 3-way merge for Cargo.lock
gitwasm: [on ] secret-scan.wasm       block commits containing credentials
gitwasm: [off] commit-lint.wasm       enforce conventional commit messages (opt-in)
gitwasm: installed
gitwasm: done — commit .gitwasm/ and .gitattributes to share this with every clone
```

From that moment: `package-lock.json` and `Cargo.lock` never conflict again,
leaked credentials never reach history — for **everyone who clones the repo**,
after one `gitwasm install`.

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

## Quickstart

```sh
rustup target add wasm32-wasip1
cargo build --release -p gitwasm      # stock modules are compiled + embedded
./demo/run-demo.sh                    # or demo\run-demo.ps1 on Windows
```

The demo proves four things end-to-end in a throwaway repo: npm lockfile
merges that always conflict in git merge cleanly; concurrent `Cargo.lock`
bumps resolve to the higher version; a staged AWS key blocks the commit; and
opt-in commit-lint rejects non-conventional messages.

In your own repo: `gitwasm init`, review what it wrote, commit. Collaborators
run `gitwasm install` once per clone (pure git config — nothing ever runs
implicitly on clone).

## Layout

```
crates/gitwasm/           host CLI (wasmtime embed): init / install / list / hook / merge / run
modules/lockfile-merge/   structural 3-way JSON merge (package-lock.json, ...)
modules/cargo-lock-merge/ structural 3-way merge for Cargo.lock
modules/secret-scan/      pre-commit scanner over the staged-tree snapshot
modules/commit-lint/      conventional-commit linter (commit-msg hook, opt-in)
demo/                     end-to-end demos (sh + ps1), run in CI on all 3 OSes
SPEC.md                   the .gitwasm/ convention — written for second implementations
SECURITY.md               exact sandbox guarantees and non-guarantees
```

A consuming repo commits only `.gitwasm/` (manifest + wasm blobs) and
`.gitattributes` lines — see [SPEC.md](SPEC.md) for the format and the module
ABI (any language that targets `wasm32-wasip1` can implement a module).

## How execution works

`gitwasm hook <name>` materializes the *staged* tree (what is actually about
to be committed) into a temp dir and mounts it **read-only** as the module's
entire world; message hooks additionally get the message as `COMMIT_MSG`.
`gitwasm merge` mounts a temp dir containing exactly `base`/`ours`/`theirs`;
the module writes `result`; nonzero exit leaves a normal git conflict for the
human. Every run is fuel- and memory-limited.

## Signing

`gitwasm keygen` once, `gitwasm sign` after changing `.gitwasm/`: every file
(including the hook shims) is hashed and ed25519-signed. Collaborators' clones
pin the signing key at `gitwasm install`; from then on tampered or unsigned
`.gitwasm/` content **refuses to execute**. Details in [SPEC.md](SPEC.md) §6
and [SECURITY.md](SECURITY.md).

## Roadmap

- More drivers: `yarn.lock`, `poetry.lock`; tree-sitter semantic
  merge for source files.
- WASI 0.2 component-model module interface (typed I/O) alongside preview1.
- **Deterministic, memoized checks**: every run is a pure function
  `hash(module) + hash(tree) → verdict`, so results are cacheable and
  trustlessly shareable — the long-term road to CI that never re-runs
  anything anyone has already run.
- **Upstream**: the goal is not this tool — it is `.gitwasm/` as an open
  convention git hosts understand and, eventually, native sandboxed-module
  support in git itself. This repo is the reference implementation.

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at
your option. Contributions are welcome under the same terms — see
[CONTRIBUTING.md](CONTRIBUTING.md).
