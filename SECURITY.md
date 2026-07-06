# Security model

gitwasm's core claim is: **it is safe to run behavior committed to a repo you
do not trust.** That claim deserves precision. This document says exactly
what the sandbox guarantees, what it does not, and what remains your problem.

## What a module can do

A module executes under wasmtime with, at most:

- read (hooks) or read/write (merge drivers) access to **one temporary
  directory** containing copies of the specific inputs for that run — never
  your working tree, never your home directory, never the `.git` directory;
- write access to your terminal via stdout/stderr — **captured and sanitized**
  (control and escape bytes stripped) so a module cannot inject terminal
  escape sequences;
- a bounded amount of CPU (fuel metering), memory (linear-memory cap), and
  wall-clock time (epoch deadline), configurable in the manifest, so a
  hostile module cannot even spin your CPU, exhaust your RAM, or stall
  forever.

## What a module cannot do

No network. No environment variables. No filesystem outside the mount. No
spawning processes. No reading your SSH keys, your browser profile, or the
rest of the repo. These are not policies — the capabilities simply do not
exist inside the sandbox (WASI capability model + wasmtime enforcement).

## What the sandbox does NOT protect you from

Honesty matters more than marketing here:

1. **Malicious verdicts.** A hostile merge driver can produce a *wrong merge
   result*; a hostile hook can block your commits or let bad ones pass. The
   sandbox contains the blast radius to the repo's own content — which is
   already fully controlled by whoever writes to the repo. Changes to
   `.gitwasm/` are ordinary committed files: review them in PRs like any code,
   and use signing (below) so unsigned changes cannot execute at all.
2. **wasmtime bugs.** The sandbox is as strong as wasmtime's isolation, which
   is industry-grade and fuzzed, but not a mathematical guarantee.
3. **Activation is explicit by design.** Nothing runs on `git clone`. Until
   you run `gitwasm install`, a cloned repo's modules are inert bytes.
4. **Local cache tampering.** Verdict replay is a local cache optimization, not
   proof by itself. A verdict becomes evidence only when `gitwasm audit`
   re-derives it from stored module bytes and input blobs. Future imported
   verdicts must not be treated as replay-eligible proof until audited or
   explicitly trusted.

## Signing

`gitwasm sign` writes `.gitwasm/signatures.toml`: sha256 hashes of every file
under `.gitwasm/` — including the hook shims git executes natively — plus an
ed25519 signature (see SPEC.md §6). `gitwasm install` pins the valid signing
keys into local git config (trust-on-first-use: activation is the trust
decision, pinning makes it durable). From then on **every** hook and merge
run verifies fail-closed: tampered, unsigned, or unpinned-key content refuses
to execute; key rotation requires an explicit `gitwasm trust`.

Honest limits of this design: TOFU means the *first* install trusts whatever
key signed the repo at that moment — it protects against later substitution,
not against a repo that was hostile from the start (the sandbox is the
defense there). Private keys live unencrypted in the maintainer's home
directory for now (`gitwasm keygen`).

## Planned hardening

- Key rotation signed by the outgoing key (today rotation is manual re-trust).
- Encrypted / hardware-backed signing keys.

## Reporting

Please report suspected sandbox escapes or contract violations privately to
the maintainers (see repository owners) before public disclosure.
