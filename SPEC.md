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

The reference CLI exposes setup profiles such as `gitwasm init lockfiles`,
`gitwasm init hooks`, and `gitwasm init all`. These are scaffolding
affordances only. They choose which stock modules and `.gitattributes` lines to
write; they do not change the manifest format described below.

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

A merge module may use either module ABI (§5); the host picks the calling
convention from the blob's format, so a manifest rule need not say which.

**preview1 modules** receive a writable mount containing exactly three files:
`base`, `ours`, `theirs` (the common ancestor and the two sides; absent sides
are empty files). argv is:

```
argv[0] = module file name
argv[1] = "base"    argv[2] = "ours"    argv[3] = "theirs"
argv[4] = "result"  argv[5] = repo-relative path being merged
```

On a successful merge the module writes the merged content to `result` and
exits 0. A nonzero exit (or missing `result`) means a genuine conflict: the
host leaves the file conflicted for the human, exactly as git would.

**component modules** (§5.2) receive nothing mounted: the host calls the
exported `merge3` function with the three sides as byte lists and the path as
a string, and reads back a typed `result` — `ok(bytes)` is the merged content,
`err(reason)` is a genuine conflict. Semantics are identical to the preview1
case; only the transport differs.

## 5. Module format

A `.wasm` blob is one of two kinds. A host distinguishes them by the eight-byte
wasm preamble: both begin with the `\0asm` magic, but the two bytes following
the version word (the "layer") are `00 00` for a core module and `01 00` for a
component. Both kinds coexist in one repository; a merge rule (§4.2) may point
at either.

### 5.1 preview1 command modules

A `_start` export and `wasi_snapshot_preview1` imports only. Any language that
compiles to `wasm32-wasip1` (Rust, C, Go/TinyGo, Zig, ...) can produce one.
This is the original ABI; it drives both hooks (§4.1) and merge drivers (§4.2).

### 5.2 component modules (WASI 0.2)

A merge driver may instead be a **component** exporting the typed world in
`wit/driver.wit`:

```wit
package gitwasm:merge@0.1.0;

interface driver {
  merge3: func(
    base: list<u8>, ours: list<u8>, theirs: list<u8>, path: string,
  ) -> result<list<u8>, string>;
}

world merge-driver { export driver; }
```

The defining property is what the world does **not** import: nothing. A
conforming component cannot name the filesystem, argv, environment, clock, or
stdio — those capabilities are absent from its instance, so the host
instantiates it with an empty linker and mounts no directory at all. This is a
strictly stronger sandbox than §5.1. Any language whose toolchain targets the
component model (Rust via `wit-bindgen`, and increasingly C, Go, and others)
can produce one; the reference `lineset-merge` module is built by compiling to
`wasm32-unknown-unknown` and wrapping the result with `wit_component`.

Hooks remain preview1-only in v0; a typed hook world is future work.

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
content-addressable — `hash(module) + hash(inputs) → verdict` (§8).

## 8. Verdicts

Because §7 makes every run a pure function of content-addressed inputs, a host
MAY record each run as a *verdict* and replay it instead of re-executing an
identical computation. A verdict is

```
key = sha256(domain ‖ kind ‖ module_hash ‖ input_hashes… ‖ path ‖ engine)
    → { exit_code, result_hash? }
```

with the module bytes and every input stored content-addressed, so a verdict is
self-contained and reproducible. The reference host keys a merge on
`(module, base, ours, theirs, path, engine)` and stores verdicts and blobs under
`<git-dir>/gitwasm/` (a per-clone cache; `gitwasm verdicts` lists them).

Replay is an optimization over locally recorded state. Audit is the trust operation:
a host re-runs the module with the stored inputs and accepts the verdict only if
the result reproduces exactly.

Two properties follow, and a conforming host MUST preserve both:

- **Replay is sound** — a host may skip execution on a key hit only if the stored
  result still hashes to the recorded `result_hash`, so a corrupted cache is
  caught rather than replayed.
- **Verdicts are re-derivable** — everything needed to reproduce a verdict is in
  the store, so a host MUST be able to re-run it and confirm the record; one that
  does not reproduce MUST be reported as failed (`gitwasm audit`). This is what
  lets a verdict be trusted without trusting whoever computed it.

The `engine` component scopes a verdict to a runtime version, since a runtime
change could alter behavior. Component modules (§5.2) import nothing and are
therefore deterministic by construction — the ideal verdict producers.

Distributing verdicts between clones — so a team or CI computes each unique
`(check, content)` pair once — is a compatible extension: verdicts are ordinary
content-addressed records and can travel through a git ref.
