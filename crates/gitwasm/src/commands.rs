use crate::gitutil::{
    git_bytes, git_config_all, git_dir, git_ignore_failure, git_string, repo_root,
};
use crate::manifest::{Limits, Manifest, GITWASM_DIR, MANIFEST_FILE};
use crate::runner::{self, run_module, run_module_bytes, MergeResult, Sandbox};
use crate::signing::{self, VerifyOutcome};
use crate::stock;
use crate::verdict::{self, MergeInputs, Store, Verdict};
use anyhow::{bail, Context, Result};
use std::fs;
use std::path::Path;

const TRUSTED_KEY_CONFIG: &str = "gitwasm.trustedkey";

fn parse_init_profile(arg: Option<&str>) -> Result<stock::InitProfile> {
    match arg.unwrap_or("all") {
        "all" => Ok(stock::InitProfile::All),
        "lockfiles" => Ok(stock::InitProfile::Lockfiles),
        "hooks" => Ok(stock::InitProfile::Hooks),
        other => bail!("unknown init profile '{other}' — expected one of: all, lockfiles, hooks"),
    }
}

fn gitwasm_hooks_path(root: &Path) -> String {
    root.join(GITWASM_DIR)
        .join("hooks")
        .to_string_lossy()
        .replace('\\', "/")
}

fn trim_trailing_slashes(value: &str) -> &str {
    value.trim_end_matches(['/', '\\'])
}

fn is_gitwasm_hooks_path(root: &Path, value: &str) -> bool {
    let value = trim_trailing_slashes(value);
    let normalized = value.replace('\\', "/");
    normalized == ".gitwasm/hooks" || normalized == trim_trailing_slashes(&gitwasm_hooks_path(root))
}

fn exact_config_value_regex(value: &str) -> String {
    let mut regex = String::from("^");
    for ch in value.chars() {
        if matches!(
            ch,
            '\\' | '.' | '^' | '$' | '*' | '+' | '?' | '(' | ')' | '[' | ']' | '{' | '}' | '|'
        ) {
            regex.push('\\');
        }
        regex.push(ch);
    }
    regex.push('$');
    regex
}

fn clear_stale_gitwasm_hooks_path(root: &Path) -> Result<bool> {
    let mut cleared = false;
    for hooks_path in git_config_all(root, "core.hooksPath")? {
        if is_gitwasm_hooks_path(root, &hooks_path) {
            let value_regex = exact_config_value_regex(&hooks_path);
            git_ignore_failure(
                root,
                &["config", "--unset-all", "core.hooksPath", &value_regex],
            );
            cleared = true;
        }
    }
    Ok(cleared)
}

/// Fail-closed signature enforcement, called before any module runs.
/// A clone that never pinned keys runs unsigned repos as before; once keys
/// are pinned (at install/trust time), unsigned or tampered `.gitwasm/`
/// content refuses to execute.
fn enforce_trust(root: &Path) -> Result<()> {
    let trusted = git_config_all(root, TRUSTED_KEY_CONFIG)?;
    if trusted.is_empty() {
        return Ok(());
    }
    match signing::verify_dir(&root.join(GITWASM_DIR))? {
        VerifyOutcome::Valid(keys) if keys.iter().any(|k| trusted.contains(k)) => Ok(()),
        VerifyOutcome::Valid(keys) => bail!(
            ".gitwasm/ is signed, but by no key this clone trusts (signers: {}). \
             If a key rotation is expected, review the change and run `gitwasm trust`",
            keys.iter()
                .map(|k| signing::fingerprint(k))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        VerifyOutcome::Unsigned => bail!(
            "this clone pins trusted signing keys but .gitwasm/ is unsigned — refusing to run modules"
        ),
        VerifyOutcome::Invalid(reason) => bail!(
            ".gitwasm/ failed signature verification ({reason}) — refusing to run modules"
        ),
    }
}

/// Pin the currently-valid signing keys into local git config (TOFU).
fn pin_signers(root: &Path) -> Result<()> {
    match signing::verify_dir(&root.join(GITWASM_DIR))? {
        VerifyOutcome::Valid(keys) => {
            let trusted = git_config_all(root, TRUSTED_KEY_CONFIG)?;
            for key in &keys {
                if !trusted.contains(key) {
                    git_string(root, &["config", "--add", TRUSTED_KEY_CONFIG, key])?;
                }
                println!(
                    "gitwasm: trusting signing key {} for this clone",
                    signing::fingerprint(key)
                );
            }
        }
        VerifyOutcome::Unsigned => {
            println!(
                "gitwasm: note: .gitwasm/ is unsigned (maintainers: `gitwasm keygen` + `gitwasm sign`)"
            );
        }
        VerifyOutcome::Invalid(reason) => bail!(
            ".gitwasm/ has a signatures.toml that does NOT verify ({reason}) — \
             refusing to activate; inspect the repo before proceeding"
        ),
    }
    Ok(())
}

/// Scaffold `.gitwasm/` with the embedded stock modules, wire up
/// `.gitattributes`, and activate. One command from zero to protected repo.
pub fn init(profile_arg: Option<&str>) -> Result<i32> {
    let profile = parse_init_profile(profile_arg)?;
    let root = repo_root()?;
    let dir = root.join(GITWASM_DIR);
    if dir.join(MANIFEST_FILE).exists() {
        bail!(
            "{}/{} already exists — edit it directly, or delete it to re-init",
            GITWASM_DIR,
            MANIFEST_FILE
        );
    }
    fs::create_dir_all(&dir)?;

    println!("gitwasm: init profile '{}'", profile.name());
    for module in stock::modules_for(profile) {
        fs::write(dir.join(module.file), module.bytes)?;
        let state = if profile == stock::InitProfile::Lockfiles || module.default_on {
            "on "
        } else {
            "off"
        };
        println!("gitwasm: [{state}] {: <22} {}", module.file, module.summary);
    }
    fs::write(
        dir.join(MANIFEST_FILE),
        stock::default_manifest_for(profile),
    )?;

    // Append (never clobber) the merge-driver attributes.
    let attributes = root.join(".gitattributes");
    let existing = fs::read_to_string(&attributes).unwrap_or_default();
    let mut additions = String::new();
    for line in stock::gitattributes_lines_for(profile) {
        if !existing.lines().any(|l| l.trim() == line) {
            additions.push_str(&line);
            additions.push('\n');
        }
    }
    if !additions.is_empty() {
        let mut content = existing;
        if !content.is_empty() && !content.ends_with('\n') {
            content.push('\n');
        }
        content.push_str(&additions);
        fs::write(&attributes, content)?;
        println!("gitwasm: updated .gitattributes");
    }

    install()?;
    println!("gitwasm: done — commit .gitwasm/ and .gitattributes to share this with every clone");
    Ok(0)
}

/// Activate the repo's committed `.gitwasm/` modules in this clone.
/// This is the only per-clone step, and it's pure git config — the
/// behavior itself travels with the repository.
pub fn install() -> Result<i32> {
    let root = repo_root()?;
    let manifest = Manifest::load(&root)?;

    let hooks_configured = !manifest.hooks.is_empty();
    let stale_hooks_path_cleared = if hooks_configured {
        let hooks_dir = root.join(GITWASM_DIR).join("hooks");
        fs::create_dir_all(&hooks_dir)?;
        for hook_name in manifest.hooks.keys() {
            let shim = hooks_dir.join(hook_name);
            // sh shim, LF endings — git for Windows runs hooks through its bundled sh.
            // Fails open when gitwasm isn't on PATH: a collaborator without the
            // tool gets a warning, not an unusable repo.
            fs::write(
                &shim,
                format!(
                    "#!/bin/sh\n\
                     if command -v gitwasm >/dev/null 2>&1; then\n\
                     \x20 exec gitwasm hook {hook_name} \"$@\"\n\
                     fi\n\
                     echo \"gitwasm: not on PATH; skipping {hook_name} hook (see .gitwasm/)\" >&2\n"
                ),
            )?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&shim, fs::Permissions::from_mode(0o755))?;
            }
            println!(
                "gitwasm: hook shim  {hook_name} -> {}",
                manifest.hooks[hook_name]
            );
        }

        let hooks_path = gitwasm_hooks_path(&root);
        git_string(&root, &["config", "core.hooksPath", &hooks_path])?;
        false
    } else if clear_stale_gitwasm_hooks_path(&root)? {
        println!("gitwasm: cleared stale core.hooksPath for no-hook manifest");
        true
    } else {
        println!("gitwasm: no hooks enabled by this manifest");
        false
    };
    git_string(
        &root,
        &[
            "config",
            "merge.gitwasm.driver",
            "gitwasm merge %O %A %B %P",
        ],
    )?;
    git_string(
        &root,
        &[
            "config",
            "merge.gitwasm.name",
            "gitwasm sandboxed wasm merge driver",
        ],
    )?;

    for rule in &manifest.merge {
        println!("gitwasm: merge rule {} -> {}", rule.pattern, rule.module);
    }
    pin_signers(&root)?;
    if hooks_configured {
        println!("gitwasm: installed (core.hooksPath + merge.gitwasm.driver set for this clone)");
    } else if stale_hooks_path_cleared {
        println!(
            "gitwasm: installed (merge.gitwasm.driver set for this clone; stale core.hooksPath cleared)"
        );
    } else {
        println!("gitwasm: installed (merge.gitwasm.driver set for this clone; no hooks enabled)");
    }
    Ok(0)
}

/// Generate a signing keypair (stored outside any repo).
pub fn keygen() -> Result<i32> {
    let path = signing::key_path()?;
    let key = signing::generate_key(&path)?;
    let public = hex::encode(key.verifying_key().to_bytes());
    println!(
        "gitwasm: new signing key {} at {}",
        signing::fingerprint(&public),
        path.display()
    );
    println!("gitwasm: public key: {public}");
    Ok(0)
}

/// Sign the current contents of `.gitwasm/` and pin our own key locally.
pub fn sign() -> Result<i32> {
    let root = repo_root()?;
    let dir = root.join(GITWASM_DIR);
    if !dir.join(MANIFEST_FILE).exists() {
        bail!(
            "no {}/{} to sign — run `gitwasm init` first",
            GITWASM_DIR,
            MANIFEST_FILE
        );
    }
    let key = signing::load_key()?;
    let files = signing::collect_files(&dir)?;
    fs::write(
        dir.join(signing::SIGNATURES_FILE),
        signing::render_signatures(&files, &key),
    )?;
    println!(
        "gitwasm: signed {} file(s) with key {}",
        files.len(),
        signing::fingerprint(&hex::encode(key.verifying_key().to_bytes()))
    );
    pin_signers(&root)?;
    println!(
        "gitwasm: commit {}/{} to publish",
        GITWASM_DIR,
        signing::SIGNATURES_FILE
    );
    Ok(0)
}

/// Explicit verification (also for CI). Exit 1 on tampered/invalid content.
pub fn verify() -> Result<i32> {
    let root = repo_root()?;
    let trusted = git_config_all(&root, TRUSTED_KEY_CONFIG)?;
    match signing::verify_dir(&root.join(GITWASM_DIR))? {
        VerifyOutcome::Valid(keys) => {
            for key in &keys {
                let status = if trusted.contains(key) {
                    "trusted by this clone"
                } else {
                    "NOT pinned here"
                };
                println!(
                    "gitwasm: valid signature by {} ({status})",
                    signing::fingerprint(key)
                );
            }
            Ok(0)
        }
        VerifyOutcome::Unsigned => {
            println!("gitwasm: .gitwasm/ is unsigned");
            Ok(0)
        }
        VerifyOutcome::Invalid(reason) => {
            eprintln!("gitwasm: VERIFICATION FAILED: {reason}");
            Ok(1)
        }
    }
}

/// Re-pin trust to the current (valid) signers — for legitimate key rotation.
pub fn trust() -> Result<i32> {
    let root = repo_root()?;
    match signing::verify_dir(&root.join(GITWASM_DIR))? {
        VerifyOutcome::Valid(_) => {
            git_ignore_failure(&root, &["config", "--unset-all", TRUSTED_KEY_CONFIG]);
            pin_signers(&root)?;
            Ok(0)
        }
        VerifyOutcome::Unsigned => bail!("nothing to trust: .gitwasm/ is unsigned"),
        VerifyOutcome::Invalid(reason) => {
            bail!("refusing to trust content that fails verification: {reason}")
        }
    }
}

/// Show what the current repo's manifest activates.
pub fn list() -> Result<i32> {
    let root = repo_root()?;
    let manifest = Manifest::load(&root)?;
    if manifest.hooks.is_empty() && manifest.merge.is_empty() {
        println!("gitwasm: no manifest (run `gitwasm init` to scaffold)");
        return Ok(0);
    }
    for (hook, module) in &manifest.hooks {
        println!("hook   {hook: <14} -> {module}");
    }
    for rule in &manifest.merge {
        println!("merge  {: <14} -> {}", rule.pattern, rule.module);
    }
    println!(
        "limits fuel={} memory={}MiB",
        manifest.limits.fuel,
        manifest.limits.memory_bytes / (1024 * 1024)
    );
    Ok(0)
}

/// Dispatch a git hook to its wasm module. The module gets a read-only
/// snapshot of the *staged* tree (what is actually about to be committed),
/// not the working tree. For message hooks (commit-msg, prepare-commit-msg)
/// the message file is copied in as COMMIT_MSG and passed as argv[1].
pub fn hook(name: &str, hook_args: &[String]) -> Result<i32> {
    let root = repo_root()?;
    enforce_trust(&root)?;
    let manifest = Manifest::load(&root)?;
    let Some(module_name) = manifest.hooks.get(name) else {
        return Ok(0); // no module registered for this hook — allow
    };
    let module = Manifest::module_path(&root, module_name);

    let tmp = tempfile::tempdir().context("creating staging snapshot dir")?;
    let listing = git_bytes(
        &root,
        &["diff", "--cached", "--name-only", "--diff-filter=ACM", "-z"],
    )?;
    let listing = String::from_utf8_lossy(&listing);
    let mut file_count = 0usize;
    for path in listing.split('\0').filter(|p| !p.is_empty()) {
        let content = git_bytes(&root, &["show", &format!(":{path}")])
            .with_context(|| format!("reading staged blob {path}"))?;
        let dest = tmp.path().join(path);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&dest, content)?;
        file_count += 1;
    }

    let mut argv = vec![module_name.clone()];
    let is_msg_hook = matches!(name, "commit-msg" | "prepare-commit-msg");
    if is_msg_hook {
        let msg_file = hook_args
            .first()
            .context("message hook invoked without a message file argument")?;
        fs::copy(root.join(msg_file), tmp.path().join("COMMIT_MSG"))
            .or_else(|_| fs::copy(msg_file, tmp.path().join("COMMIT_MSG")))
            .context("copying commit message into sandbox")?;
        argv.push("COMMIT_MSG".into());
    } else if file_count == 0 {
        return Ok(0);
    }

    eprintln!(
        "gitwasm: {name} -> {module_name} ({file_count} staged file(s), sandboxed read-only)"
    );
    run_module(
        &module,
        Sandbox {
            dir: tmp.path(),
            writable: false,
            argv,
            limits: manifest.limits,
        },
    )
}

/// The outcome of running a merge module, independent of ABI: the merged bytes,
/// or a genuine conflict the host leaves for the human.
enum MergeOutcome {
    Clean(Vec<u8>),
    Conflict,
}

/// Git merge driver entry point: `gitwasm merge %O %A %B %P`.
/// %O/%A/%B are temp files (base/ours/theirs), %P is the repo-relative path.
/// On success the merged result must be left in %A.
///
/// A merge is a pure function of `(module, base, ours, theirs, path)`, so the
/// run is memoized as a verdict: an identical computation replays its recorded
/// result instead of re-executing, and the record can be re-derived on demand
/// (`gitwasm audit`). See verdict.rs.
pub fn merge(base: &str, ours: &str, theirs: &str, path: &str) -> Result<i32> {
    let root = repo_root()?;
    enforce_trust(&root)?;
    let manifest = Manifest::load(&root)?;
    let Some(module_name) = manifest.merge_module(path) else {
        eprintln!("gitwasm: no merge module matches '{path}' — leaving conflict for git");
        return Ok(1);
    };
    let module = Manifest::module_path(&root, module_name);
    let module_bytes =
        fs::read(&module).with_context(|| format!("reading {}", module.display()))?;

    // git may pass a non-existent path for an absent side (no common ancestor).
    let read_side = |p: &str| fs::read(p).unwrap_or_default();
    let (base_b, ours_b, theirs_b) = (read_side(base), read_side(ours), read_side(theirs));

    let inputs = MergeInputs {
        base: verdict::sha256_hex(&base_b),
        ours: verdict::sha256_hex(&ours_b),
        theirs: verdict::sha256_hex(&theirs_b),
    };
    let key = verdict::merge_key(&verdict::sha256_hex(&module_bytes), &inputs, path);
    let store = verdict_store(&root);

    if let Some(store) = &store {
        if let Some(recorded) = store.get(&key)? {
            if let Some(code) = replay_merge(store, &recorded, ours, path, module_name)? {
                return Ok(code);
            }
            // A damaged cache entry falls through to a fresh run.
        }
    }

    let outcome = run_merge(
        &module_bytes,
        module_name,
        (&base_b, &ours_b, &theirs_b),
        path,
        manifest.limits,
        true,
    )?;

    if let Some(store) = &store {
        record_merge(
            store,
            &key,
            &module_bytes,
            (&base_b, &ours_b, &theirs_b),
            path,
            &outcome,
        )
        .unwrap_or_else(|e| eprintln!("gitwasm: note: could not record verdict ({e:#})"));
    }

    match outcome {
        MergeOutcome::Clean(result) => {
            fs::write(ours, result).context("writing merge result back to %A")?;
            Ok(0)
        }
        MergeOutcome::Conflict => Ok(1),
    }
}

/// Run a merge module on in-memory sides, dispatching on ABI. This is the single
/// primitive shared by the live merge and by `gitwasm audit`'s re-derivation.
fn run_merge(
    module_bytes: &[u8],
    module_name: &str,
    sides: (&[u8], &[u8], &[u8]),
    path: &str,
    limits: Limits,
    announce: bool,
) -> Result<MergeOutcome> {
    let (base, ours, theirs) = sides;
    if runner::is_component(module_bytes) {
        if announce {
            eprintln!(
                "gitwasm: merging '{path}' with {module_name} (sandboxed component, no mount)"
            );
        }
        match runner::run_component_merge(module_bytes, base, ours, theirs, path, limits)? {
            MergeResult::Merged(bytes) => Ok(MergeOutcome::Clean(bytes)),
            MergeResult::Conflict(reason) => {
                if announce {
                    eprintln!("gitwasm: component reports a real conflict for '{path}': {reason}");
                }
                Ok(MergeOutcome::Conflict)
            }
        }
    } else {
        if announce {
            eprintln!("gitwasm: merging '{path}' with {module_name} (sandboxed)");
        }
        let tmp = tempfile::tempdir().context("creating merge sandbox dir")?;
        fs::write(tmp.path().join("base"), base)?;
        fs::write(tmp.path().join("ours"), ours)?;
        fs::write(tmp.path().join("theirs"), theirs)?;
        let code = run_module_bytes(
            module_bytes,
            Sandbox {
                dir: tmp.path(),
                writable: true,
                argv: vec![
                    module_name.to_string(),
                    "base".into(),
                    "ours".into(),
                    "theirs".into(),
                    "result".into(),
                    path.to_string(),
                ],
                limits,
            },
        )?;
        let result = tmp.path().join("result");
        if code == 0 && result.exists() {
            Ok(MergeOutcome::Clean(fs::read(result)?))
        } else {
            if announce {
                eprintln!("gitwasm: module reported a real conflict for '{path}'");
            }
            Ok(MergeOutcome::Conflict)
        }
    }
}

/// The verdict store for this clone, unless disabled. Never fatal — a merge must
/// still work if the cache can't be opened.
fn verdict_store(root: &Path) -> Option<Store> {
    if std::env::var_os("GITWASM_NO_VERDICTS").is_some() {
        return None;
    }
    match git_dir(root).and_then(|dir| Store::open(&dir)) {
        Ok(store) => Some(store),
        Err(e) => {
            eprintln!("gitwasm: note: verdict cache unavailable ({e:#})");
            None
        }
    }
}

/// Replay a recorded verdict without running the module. Returns the exit code
/// to use, or `None` if the entry is unusable (missing/corrupt result) so the
/// caller re-runs from scratch.
fn replay_merge(
    store: &Store,
    verdict: &Verdict,
    ours: &str,
    path: &str,
    module_name: &str,
) -> Result<Option<i32>> {
    match &verdict.result {
        Some(result_hash) => match store.get_blob(result_hash) {
            Ok(bytes) => {
                fs::write(ours, bytes).context("writing memoized result to %A")?;
                eprintln!(
                    "gitwasm: memoized '{path}' via {module_name} → verdict {} (clean; `gitwasm audit` re-derives)",
                    &verdict.key[..12]
                );
                Ok(Some(0))
            }
            Err(_) => Ok(None),
        },
        None => {
            eprintln!(
                "gitwasm: memoized '{path}' via {module_name} → verdict {} (conflict)",
                &verdict.key[..12]
            );
            Ok(Some(verdict.exit_code.max(1)))
        }
    }
}

/// Record a fresh merge computation: store the module, the three sides, and the
/// result content-addressed, then the verdict that ties them together.
fn record_merge(
    store: &Store,
    key: &str,
    module_bytes: &[u8],
    sides: (&[u8], &[u8], &[u8]),
    path: &str,
    outcome: &MergeOutcome,
) -> Result<()> {
    let inputs = MergeInputs {
        base: store.put_blob(sides.0)?,
        ours: store.put_blob(sides.1)?,
        theirs: store.put_blob(sides.2)?,
    };
    let (exit_code, result) = match outcome {
        MergeOutcome::Clean(bytes) => (0, Some(store.put_blob(bytes)?)),
        MergeOutcome::Conflict => (1, None),
    };
    store.put(&Verdict {
        key: key.to_string(),
        kind: "merge".into(),
        module: store.put_blob(module_bytes)?,
        path: path.to_string(),
        exit_code,
        result,
        engine: verdict::ENGINE_ID.to_string(),
        inputs,
    })
}

/// List the verdicts recorded in this clone's cache.
pub fn verdicts() -> Result<i32> {
    let root = repo_root()?;
    let store = Store::open(&git_dir(&root)?)?;
    let recorded = store.list()?;
    if recorded.is_empty() {
        println!("gitwasm: no verdicts recorded yet — they accrue as modules run");
        return Ok(0);
    }
    for verdict in &recorded {
        let status = if verdict.result.is_some() {
            "clean"
        } else {
            "conflict"
        };
        println!(
            "{}  {:<6} {:<8} {}",
            &verdict.key[..12],
            verdict.kind,
            status,
            verdict.path
        );
    }
    println!(
        "gitwasm: {} verdict(s) — `gitwasm audit` re-derives every one",
        recorded.len()
    );
    Ok(0)
}

/// Re-derive recorded verdicts from their content-addressed inputs and confirm
/// each reproduces exactly. This is the trustless core of the whole idea: a
/// verdict you cannot reproduce is a verdict you have no reason to believe.
/// Exit nonzero if any verdict fails to reproduce.
pub fn audit(selector: Option<&str>) -> Result<i32> {
    let root = repo_root()?;
    let store = Store::open(&git_dir(&root)?)?;
    let limits = Manifest::load(&root)?.limits;
    let to_check = match selector {
        Some(key) => vec![store
            .get(key)?
            .with_context(|| format!("no verdict with key {key}"))?],
        None => store.list()?,
    };
    if to_check.is_empty() {
        println!("gitwasm: no verdicts to audit");
        return Ok(0);
    }

    let (mut reproduced, mut failed) = (0usize, 0usize);
    for verdict in &to_check {
        let short = &verdict.key[..12];
        match rederive_merge(&store, verdict, limits) {
            Ok(true) => {
                println!("gitwasm: {short} reproduces ({})", verdict.path);
                reproduced += 1;
            }
            Ok(false) => {
                eprintln!("gitwasm: {short} DOES NOT reproduce ({})", verdict.path);
                failed += 1;
            }
            Err(e) => {
                eprintln!("gitwasm: {short} cannot be audited: {e:#}");
                failed += 1;
            }
        }
    }
    println!("gitwasm: {reproduced} reproduced, {failed} failed");
    Ok(i32::from(failed > 0))
}

/// Re-run a merge verdict from its stored inputs and check it matches the record
/// exactly (same conflict-or-clean, same result bytes).
fn rederive_merge(store: &Store, verdict: &Verdict, limits: Limits) -> Result<bool> {
    if verdict.kind != "merge" {
        bail!("cannot audit verdict of kind '{}'", verdict.kind);
    }
    let module = store.get_blob(&verdict.module)?;
    let base = store.get_blob(&verdict.inputs.base)?;
    let ours = store.get_blob(&verdict.inputs.ours)?;
    let theirs = store.get_blob(&verdict.inputs.theirs)?;

    let outcome = run_merge(
        &module,
        "audit",
        (&base, &ours, &theirs),
        &verdict.path,
        limits,
        false,
    )?;
    Ok(match (&outcome, &verdict.result) {
        (MergeOutcome::Clean(bytes), Some(hash)) => {
            verdict.exit_code == 0 && &verdict::sha256_hex(bytes) == hash
        }
        (MergeOutcome::Conflict, None) => verdict.exit_code != 0,
        _ => false,
    })
}

/// Dev utility: run any module with the current directory preopened read-only.
pub fn run_direct(wasm: &str, args: &[String]) -> Result<i32> {
    let wasm_path = Path::new(wasm);
    if !wasm_path.exists() {
        bail!("no such module: {wasm}");
    }
    if runner::is_component(&fs::read(wasm_path)?) {
        bail!(
            "{wasm} is a component-model module — components are typed merge \
             drivers invoked via `gitwasm merge`, not `gitwasm run`"
        );
    }
    let mut argv = vec![wasm.to_string()];
    argv.extend(args.iter().cloned());
    let cwd = std::env::current_dir()?;
    run_module(
        wasm_path,
        Sandbox {
            dir: &cwd,
            writable: false,
            argv,
            limits: Default::default(),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_init_profile_defaults_to_all() {
        assert_eq!(parse_init_profile(None).unwrap(), stock::InitProfile::All);
    }

    #[test]
    fn parse_init_profile_accepts_named_profiles() {
        assert_eq!(
            parse_init_profile(Some("lockfiles")).unwrap(),
            stock::InitProfile::Lockfiles
        );
        assert_eq!(
            parse_init_profile(Some("hooks")).unwrap(),
            stock::InitProfile::Hooks
        );
        assert_eq!(
            parse_init_profile(Some("all")).unwrap(),
            stock::InitProfile::All
        );
    }

    #[test]
    fn parse_init_profile_rejects_unknown_profile() {
        let err = parse_init_profile(Some("everything")).unwrap_err();
        assert!(format!("{err:#}").contains("unknown init profile"));
        assert!(format!("{err:#}").contains("lockfiles"));
    }

    #[test]
    fn gitwasm_hooks_path_matches_absolute_managed_path() {
        let root = Path::new("/tmp/example-repo");
        assert!(is_gitwasm_hooks_path(
            root,
            "/tmp/example-repo/.gitwasm/hooks"
        ));
    }

    #[test]
    fn gitwasm_hooks_path_matches_relative_managed_path() {
        let root = Path::new("/tmp/example-repo");
        assert!(is_gitwasm_hooks_path(root, ".gitwasm/hooks"));
    }

    #[test]
    fn gitwasm_hooks_path_rejects_user_owned_path() {
        let root = Path::new("/tmp/example-repo");
        assert!(!is_gitwasm_hooks_path(root, ".githooks"));
        assert!(!is_gitwasm_hooks_path(
            root,
            "/tmp/example-repo/custom-hooks"
        ));
    }

    fn lineset_component() -> &'static [u8] {
        stock::STOCK
            .iter()
            .find(|m| m.file == "lineset-merge.wasm")
            .expect("lineset-merge is a stock module")
            .bytes
    }

    /// The full verdict cycle on the real embedded component: run → record →
    /// re-derive (matches) → tamper the record (no longer reproduces).
    #[test]
    fn merge_verdict_records_rederives_and_catches_tampering() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(tmp.path()).unwrap();
        let module = lineset_component();
        let sides = (
            b"a\n".as_slice(),
            b"a\nb\n".as_slice(),
            b"a\nc\n".as_slice(),
        );

        let inputs = MergeInputs {
            base: verdict::sha256_hex(sides.0),
            ours: verdict::sha256_hex(sides.1),
            theirs: verdict::sha256_hex(sides.2),
        };
        let key = verdict::merge_key(&verdict::sha256_hex(module), &inputs, "go.sum");

        let outcome =
            run_merge(module, "lineset", sides, "go.sum", Limits::default(), false).unwrap();
        assert!(
            matches!(outcome, MergeOutcome::Clean(_)),
            "disjoint lines merge clean"
        );
        record_merge(&store, &key, module, sides, "go.sum", &outcome).unwrap();

        let recorded = store.get(&key).unwrap().expect("verdict was recorded");
        assert!(
            rederive_merge(&store, &recorded, Limits::default()).unwrap(),
            "an honest verdict must re-derive"
        );

        // A verdict claiming a different result must fail re-derivation.
        let mut forged = recorded.clone();
        forged.result = Some(verdict::sha256_hex(b"a forgery"));
        assert!(
            !rederive_merge(&store, &forged, Limits::default()).unwrap(),
            "a forged result must not reproduce"
        );
    }
}
