# gitwasm Adoption Roadmap Design

Date: 2026-07-06

## Summary

The adoption path for gitwasm should lead with a narrow, concrete promise:

> Lockfile conflicts are over.

The broader project is a repo-committed Git behavior substrate: signed,
sandboxed WebAssembly modules for hooks and merge drivers, with reproducible
verdicts that can eventually be shared between clones and CI. That is the
magnum-opus arc, but it should not be the initial adoption pitch. The first
user-facing product should solve the pain developers already recognize:
generated dependency files conflict too often, and Git's built-in extension
points are too local and unsafe to distribute by clone.

The roadmap has three phases:

1. Conflict Killer: make generated-file merges, especially lockfiles, the
   obvious reason to install gitwasm.
2. Safe Repo Policy: expand from merge drivers into signed, sandboxed,
   repo-committed hooks and local policy checks.
3. Verdicts: turn module runs into reproducible, auditable, and eventually
   shareable facts.

Each phase must be independently valuable. Later phases should strengthen the
same story rather than changing the product into a different tool.

## Current State

The current repository already proves the core technical thesis:

- `gitwasm init`, `install`, `list`, `hook`, `merge`, `run`, `sign`, `verify`,
  `trust`, `verdicts`, and `audit` exist in the CLI surface.
- Stock merge modules exist for `package-lock.json`, `package.json`,
  `Cargo.lock`, `yarn.lock` v1, `poetry.lock`, and `go.sum`.
- Stock hooks exist for secret scanning and optional conventional commit
  linting.
- Signing covers every file under `.gitwasm/`, including native hook shims, and
  runtime execution verifies fail-closed once keys are pinned.
- WASI preview1 modules and typed WASI 0.2 component merge drivers coexist.
- `lineset-merge` is the reference component driver, using the typed merge
  world with no imports.
- Local verdict recording and audit exist for merge runs.
- The Unix and Windows demos exercise merge drivers, hooks, signing, tamper
  rejection, and verdict audit.

The main adoption gaps are not the sandbox core. They are product shape:

- There is no phase-specific `gitwasm init lockfiles` command.
- `pnpm-lock.yaml` is not supported yet.
- Release/install paths exist, but the user-facing installation story is not
  the centerpiece of the docs.
- Verdict replay currently behaves like a local cache. Audit is the proof
  operation. That distinction must be made explicit before verdicts are shared
  across clones.
- The README contains the platform vision, but the front-door message should
  emphasize one immediate win before explaining the substrate.

## Phase 1: Conflict Killer

### Goal

Make gitwasm adoptable for a team that only cares about reducing merge pain.
The user should be able to understand the value in under one minute:

1. Ordinary Git conflicts on generated dependency files.
2. `gitwasm init lockfiles`.
3. The same conflict resolves structurally and reproducibly.
4. The behavior is committed to the repo and works on every collaborator's
   platform after `gitwasm install`.

### Scope

Phase 1 focuses on merge drivers for generated dependency and line-set files.
It includes:

- `gitwasm init lockfiles`, which scaffolds only the stock merge drivers and
  `.gitattributes` entries needed for generated-file merges.
- Continued support for `package-lock.json`, `package.json`, `Cargo.lock`,
  `yarn.lock` v1, `poetry.lock`, and `go.sum`.
- New support for `pnpm-lock.yaml`.
- Diagnostics that explain whether a merge was clean, intentionally biased
  toward one side, or refused as a genuine conflict.
- A short demo script and documentation path centered on lockfile conflicts.
- A GitHub Action or documented CI recipe that verifies `.gitwasm/`, builds the
  binary, runs the end-to-end demo, and audits local verdicts.

### Non-Goals

Phase 1 does not need to solve arbitrary semantic source merges. It also does
not need distributed verdicts. The product can record local verdicts, but the
adoption promise is clean generated-file merges, not cache distribution.

### Acceptance Criteria

- A new repo can run `gitwasm init lockfiles` and commit `.gitwasm/` plus
  `.gitattributes` without enabling hooks.
- A clone that runs `gitwasm install` gets the same merge behavior on Linux,
  macOS, Windows, and CI.
- The demo shows at least three lockfile/generated-file conflicts resolving
  cleanly: npm, Cargo, and one of Poetry, Yarn, pnpm, or Go.
- Unsupported or ambiguous lockfile formats fail closed with an explanation
  rather than producing a questionable result.
- The README's first practical path presents the lockfile workflow before the
  full platform story.

## Phase 2: Safe Repo Policy

### Goal

Make gitwasm a safer way to distribute local Git policy than traditional hooks
or language-specific hook managers. The promise is:

> Repo policy you can commit, review, sign, and run in a sandbox.

This phase builds on the explicit-install trust boundary. Nothing runs on clone;
activation remains a local decision.

### Scope

Phase 2 focuses on hooks and trust UX:

- `gitwasm init hooks`, which scaffolds hook modules without merge drivers.
- A clear default hook pack: secret scan on by default, commit lint opt-in.
- Additional hook candidates only if they are deterministic and low-risk:
  generated-file freshness checks, max file size checks, license/header checks,
  or forbidden-path checks.
- `gitwasm trust status`, showing unsigned, valid, pinned, unpinned, invalid,
  and rotated-key states clearly.
- Key rotation flow that is explicit and reviewable.
- Documentation comparing gitwasm to conventional Git hooks and `pre-commit`
  without framing them as enemies. The point is the different trust and
  portability model: committed WASM modules, signatures, sandboxing, and no
  repo-specific language bootstrap.

### Non-Goals

Phase 2 does not need to replace CI. Local hooks remain preflight checks that
improve developer feedback. CI should still verify the repository's committed
state independently.

### Acceptance Criteria

- A repo can choose merge drivers, hooks, or both without hand-editing the
  default manifest.
- Signature failures and key trust states are understandable without reading
  `SPEC.md`.
- Hook runs use staged-tree snapshots, not the working tree, and the docs make
  that property explicit.
- The end-to-end demo still proves tamper rejection after hook expansion.

## Phase 3: Verdicts

### Goal

Turn sandboxed module runs into reproducible facts:

> A module applied to content-addressed inputs under a specific engine produced
> this outcome, and anyone can re-derive it.

Verdicts are the long-term differentiator. They enable memoization, audit, and
eventually CI or team-wide reuse of identical checks. They should be introduced
as a correctness and reproducibility layer before being marketed as a speed
feature.

### Scope

Phase 3 has two subphases.

First, harden local semantics:

- Keep the distinction clear: replay is a cache operation; audit is the proof
  operation.
- A corrupted result blob must never replay.
- A verdict whose metadata no longer matches its storage key or schema must be
  ignored or treated as unauditable.
- Imported or externally supplied verdicts must not become replay-eligible until
  the local host has audited them or the user has explicitly trusted their
  provenance.
- `gitwasm audit --strict` should be the command that proves all replay-eligible
  verdicts still reproduce.

Second, expand verdict coverage and transport:

- Record hook/check verdicts, not only merge verdicts.
- Add export/import mechanics for verdict bundles or a git-ref-backed verdict
  namespace.
- Keep module bytes and inputs content-addressed so audit is self-contained.
- Include the engine identifier in verdict keys so runtime changes do not
  silently reuse old results.

### Non-Goals

Phase 3 does not require trusting other developers' machines. A shared verdict
is useful only because it can be re-derived locally or accepted through an
explicit trust policy. The design should not imply that a remote cache is proof.

### Acceptance Criteria

- `gitwasm audit` can re-run every recorded merge verdict from stored module
  bytes and input blobs.
- Forged or tampered verdict metadata is detected by audit and cannot silently
  become trusted shared state.
- Hook verdicts use content-addressed staged-tree inputs.
- Imported verdicts have a visible unaudited state until audited or trusted.
- Documentation says exactly when a verdict is a cache hit, when it is proof,
  and what trust decision is being made.

## Architecture Boundaries

### CLI Profiles

The CLI should expose profile-oriented setup commands:

- `gitwasm init lockfiles`
- `gitwasm init hooks`
- `gitwasm init all`

The existing `gitwasm init` can remain as an alias for the recommended default,
but the docs should prefer explicit profiles. Profiles are product affordances,
not separate manifest formats. They choose which stock modules and
`.gitattributes` entries to write.

### Module Packs

Stock modules should be grouped by purpose:

- Merge pack: lockfile and generated-file merge drivers.
- Hook pack: secret scan, commit lint, and later deterministic policy checks.
- Experimental pack: modules that are useful but not ready as default adoption
  paths.

The manifest should remain simple. Complexity belongs in the CLI scaffolding
and docs, not in the committed repo convention.

### Runner

The runner continues to support:

- preview1 command modules for hooks and legacy/simple merge drivers;
- typed WASI 0.2 components for merge drivers that can avoid filesystem access
  entirely.

New merge modules should prefer components when practical. Hooks can remain
preview1 until a typed hook world is designed.

### Verdict Store

The verdict store should distinguish provenance and replay eligibility:

- locally recorded verdicts from module runs;
- imported unaudited verdicts;
- audited imported verdicts;
- trusted-provenance verdicts, if a future trust policy is added.

This avoids overstating what the cache proves. The store can be optimized later;
the first requirement is honest state transitions.

## Data Flow

### Merge Flow

1. Git invokes `gitwasm merge %O %A %B %P`.
2. gitwasm loads the manifest, enforces signature trust if keys are pinned, and
   selects the first matching merge rule.
3. gitwasm hashes the module and the three sides.
4. If a replay-eligible local verdict exists, gitwasm may replay it as a cache
   hit.
5. Otherwise gitwasm runs the module in the appropriate sandbox.
6. A clean result is written to `%A`; a conflict exits nonzero so Git leaves the
   conflict to the human.
7. gitwasm records the module, inputs, output, engine, and verdict metadata for
   later audit.

### Hook Flow

1. Git invokes a shim under `.gitwasm/hooks`.
2. The shim calls `gitwasm hook <name>`.
3. gitwasm verifies `.gitwasm/` signatures if trust is pinned.
4. gitwasm materializes the staged tree, not the working tree.
5. The hook module runs with a read-only mount and sanitized output.
6. In Phase 3, gitwasm can record a hook verdict keyed by module, staged-tree
   content, hook name, arguments, and engine.

### Verdict Audit Flow

1. `gitwasm audit` enumerates verdict records.
2. For each verdict, gitwasm loads module bytes and input blobs by content
   address.
3. gitwasm re-runs the module with the recorded inputs and engine-compatible
   semantics.
4. The verdict reproduces only if the conflict/clean status and result hash
   match the record.
5. Failed audit exits nonzero.

## Error Handling and Trust

gitwasm should bias toward understandable fail-closed behavior:

- Unknown or malformed lockfile syntax should refuse the merge rather than
  corrupting content.
- Signature verification failures should prevent module execution after keys
  are pinned.
- Missing `gitwasm` on `PATH` should not make a clone unusable; hook shims can
  warn and skip, while CI should enforce verification.
- Verdict cache failures should not prevent live merges; the merge can run and
  report that recording was unavailable.
- Imported verdicts should be visibly unaudited until promoted by audit or
  explicit trust.

## Testing and Verification

Each phase needs tests at the level where regressions would matter:

- Unit tests for merge algorithms, including deletion-vs-modification,
  concurrent version bumps, malformed input, deterministic output ordering, and
  unsupported-format refusal.
- Host tests for sandbox limits, output sanitization, component detection,
  component merge calls, signing verification, and verdict audit.
- End-to-end demo tests on Linux, macOS, and Windows.
- CI release checks that build the binary, build embedded stock modules, run
  tests, run the demo, and verify formatting/clippy.
- Phase 3 adversarial tests for corrupted blobs, forged verdict metadata,
  unaudited imports, and strict audit failures.

## Documentation Shape

The README should be ordered for adoption:

1. Lockfile conflict demo and quickstart.
2. Why committed Git behavior is normally unsafe.
3. Why WASM sandboxing and explicit install change the tradeoff.
4. Signing and trust.
5. Components.
6. Verdicts.
7. Specification and contributor details.

`SPEC.md` should remain implementation-oriented. `SECURITY.md` should keep the
honest-threat-model tone and explicitly distinguish sandbox safety, maintainer
trust, local cache replay, and verdict audit.

## Risks

- Over-positioning verdicts too early could make the project sound abstract
  before users feel the lockfile win.
- Weak merge drivers would damage trust quickly. Each stock driver must refuse
  questionable input rather than guess.
- If signing UX feels scary, teams may avoid enabling it even though it is a
  core differentiator.
- If verdict replay is described as proof without audit, the trust model becomes
  misleading.
- Too many stock hooks can make gitwasm feel intrusive. The default hook pack
  should stay small.

## Implementation Order

The implementation plan should start with Phase 1, not with the whole roadmap:

1. Add profile-aware `init` scaffolding.
2. Add `pnpm-lock.yaml` support with fail-closed parsing and conflict
   diagnostics.
3. Update README and demo around the lockfile conflict wedge.
4. Add CI/action guidance for verification and audit.
5. Harden verdict replay language and tests so Phase 3 claims remain honest.

Phase 2 and Phase 3 should follow only after the Phase 1 adoption path is crisp
and shippable.

## Success Criteria for the Roadmap

The roadmap is working when:

- A new user can adopt gitwasm for lockfiles without learning the whole platform.
- A security-minded reviewer can understand why explicit install, signatures,
  and sandboxing make committed behavior acceptable.
- A maintainer can explain verdicts without implying trust in remote machines.
- The demo proves the value, not just the mechanism.
- The specification stays small enough for another implementation to follow.
