# The `.gitwasm/` convention — specification (v0)

This document specifies the repo-embedded behavior convention so that
implementations other than the reference `gitwasm` CLI can exist. A standard
is a thing with more than one implementation; this is the contract they share.

Status: **draft**, versioned with this repository. Breaking changes bump the
manifest's implied version; v0 has no version field and is this document.

## 1. Repository layout

A participating repository commits:

```
.gitwasm/
  manifest.toml      required — maps extension points to modules
  <name>.wasm        one or more WASI preview1 command modules
.gitattributes       merge patterns point at the "gitwasm" driver
```

Everything under `.gitwasm/` is ordinary committed content: versioned,
diffable in PRs, and delivered by clone/fetch like any other file.

## 2. Manifest format

```toml
# .gitwasm/manifest.toml
[hooks]
pre-commit = "secret-scan.wasm"     # hook name -> module file in .gitwasm/
commit-msg = "commit-lint.wasm"

[[merge]]
pattern = "package-lock.json"       # gitattributes-style pattern
module = "lockfile-merge.wasm"

[limits]                            # optional; defaults shown
fuel = 10000000000                  # abstract instruction budget per run
memory_bytes = 536870912            # linear memory cap per run
```

Merge `pattern` semantics: a pattern containing `/` matches against the full
repo-relative path; otherwise against the basename. `*` matches any run of
characters. First matching rule wins.

## 3. Sandbox contract

A host MUST execute modules with, at most:

- **one preopened directory**, mounted at `.`, whose contents are defined per
  extension point below;
- the argv defined per extension point;
- inherited stdout/stderr (for reporting to the user);
- **no** environment variables, network, clocks beyond WASI defaults, or any
  filesystem access outside the mount;
- enforced fuel and memory limits (from `[limits]` or host defaults).

A host MUST NOT execute modules implicitly on clone. Activation is an
explicit, informed step (`gitwasm install`), and is pure local git config.

## 4. Extension point contracts

### 4.1 Hooks

The mount is a **read-only snapshot of the staged tree** (`git diff --cached`
paths, ACM filter, blob contents from the index — not the working tree).
argv[0] is the module file name.

For message hooks (`commit-msg`, `prepare-commit-msg`) the host additionally
copies the message file into the mount as `COMMIT_MSG` and passes `COMMIT_MSG`
as argv[1].

Exit code 0 allows the git operation; any other exit code aborts it.

### 4.2 Merge drivers

The mount is writable and contains exactly three files: `base`, `ours`,
`theirs` (the common ancestor and the two sides; absent sides are empty
files). argv is:

```
argv[0] = module file name
argv[1] = "base"    argv[2] = "ours"    argv[3] = "theirs"
argv[4] = "result"  argv[5] = repo-relative path being merged
```

On a successful merge the module writes the merged content to `result` and
exits 0. A nonzero exit (or missing `result`) means a genuine conflict: the
host leaves the file conflicted for the human, exactly as git would.

## 5. Module format

Modules are WebAssembly **WASI preview1 command modules**: a `_start` export,
`wasi_snapshot_preview1` imports only. Any language that compiles to
`wasm32-wasip1` (Rust, C, Go/TinyGo, Zig, ...) can produce one. A future
spec revision will add a WASI 0.2 component-model interface with typed I/O;
hosts should expect both to coexist.

## 6. Signing (optional but recommended)

`.gitwasm/signatures.toml` records a sha256 hash of **every other file under
`.gitwasm/`** (module blobs, manifest, hook shims — shims especially, since
git executes them natively) plus one or more ed25519 signatures:

```toml
[files]
"manifest.toml" = "<sha256 hex>"
"hooks/pre-commit" = "<sha256 hex>"
"secret-scan.wasm" = "<sha256 hex>"

[[signatures]]
key = "<ed25519 public key, hex>"
sig = "<ed25519 signature, hex>"
```

The signed payload is the exact byte string:

```
"gitwasm-signatures-v1\n" + for each file in lexicographic name order:
    name + "\n" + sha256hex + "\n"
```

Trust semantics: at activation (`gitwasm install`) a host pins the currently
valid signing keys into **local, per-clone** state (trust-on-first-use;
activation is already the explicit trust decision). Once keys are pinned, the
host MUST verify before every module run and MUST refuse to execute
(fail-closed) if content is unsigned, tampered, or signed only by unpinned
keys. Re-pinning after a legitimate key rotation is an explicit user action
(`gitwasm trust`).

Repos MUST carry a `.gitwasm/** -text` gitattributes line: EOL conversion
would otherwise change file hashes between platforms and break verification.

## 7. Determinism

Modules SHOULD be deterministic: given identical mounts and argv, produce
identical outputs. The contract above gives modules no ambient sources of
nondeterminism, so this holds by default. Determinism is what makes results
content-addressable — `hash(module) + hash(inputs) → verdict` — enabling
future memoization and trustless sharing of check results.
