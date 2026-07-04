use crate::gitutil::{git_bytes, git_config_all, git_ignore_failure, git_string, repo_root};
use crate::manifest::{Manifest, GITWASM_DIR, MANIFEST_FILE};
use crate::runner::{run_module, Sandbox};
use crate::signing::{self, VerifyOutcome};
use crate::stock;
use anyhow::{bail, Context, Result};
use std::fs;
use std::path::Path;

const TRUSTED_KEY_CONFIG: &str = "gitwasm.trustedkey";

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
pub fn init() -> Result<i32> {
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

    for module in stock::STOCK {
        fs::write(dir.join(module.file), module.bytes)?;
        let state = if module.default_on { "on " } else { "off" };
        println!("gitwasm: [{state}] {: <22} {}", module.file, module.summary);
    }
    fs::write(dir.join(MANIFEST_FILE), stock::default_manifest())?;

    // Append (never clobber) the merge-driver attributes.
    let attributes = root.join(".gitattributes");
    let existing = fs::read_to_string(&attributes).unwrap_or_default();
    let mut additions = String::new();
    for line in stock::gitattributes_lines() {
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

    let hooks_path = hooks_dir.to_string_lossy().replace('\\', "/");
    git_string(&root, &["config", "core.hooksPath", &hooks_path])?;
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
    println!("gitwasm: installed (core.hooksPath + merge.gitwasm.driver set for this clone)");
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

/// Git merge driver entry point: `gitwasm merge %O %A %B %P`.
/// %O/%A/%B are temp files (base/ours/theirs), %P is the repo-relative path.
/// On success the merged result must be left in %A.
pub fn merge(base: &str, ours: &str, theirs: &str, path: &str) -> Result<i32> {
    let root = repo_root()?;
    enforce_trust(&root)?;
    let manifest = Manifest::load(&root)?;
    let Some(module_name) = manifest.merge_module(path) else {
        eprintln!("gitwasm: no merge module matches '{path}' — leaving conflict for git");
        return Ok(1);
    };
    let module = Manifest::module_path(&root, module_name);

    // The module's entire world: one temp dir with exactly these three files.
    let tmp = tempfile::tempdir().context("creating merge sandbox dir")?;
    copy_or_empty(base, &tmp.path().join("base"))?;
    copy_or_empty(ours, &tmp.path().join("ours"))?;
    copy_or_empty(theirs, &tmp.path().join("theirs"))?;

    eprintln!("gitwasm: merging '{path}' with {module_name} (sandboxed)");
    let code = run_module(
        &module,
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
            limits: manifest.limits,
        },
    )?;

    let result = tmp.path().join("result");
    if code == 0 && result.exists() {
        fs::copy(&result, ours).context("writing merge result back to %A")?;
        Ok(0)
    } else {
        eprintln!("gitwasm: module reported a real conflict for '{path}'");
        Ok(1)
    }
}

/// Dev utility: run any module with the current directory preopened read-only.
pub fn run_direct(wasm: &str, args: &[String]) -> Result<i32> {
    let wasm_path = Path::new(wasm);
    if !wasm_path.exists() {
        bail!("no such module: {wasm}");
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

fn copy_or_empty(src: &str, dest: &Path) -> Result<()> {
    if Path::new(src).exists() {
        fs::copy(src, dest).with_context(|| format!("copying {src}"))?;
    } else {
        fs::write(dest, b"")?;
    }
    Ok(())
}
