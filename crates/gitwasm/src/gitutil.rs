use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn git_bytes(cwd: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let out = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .context("failed to spawn git")?;
    if !out.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(out.stdout)
}

pub fn git_string(cwd: &Path, args: &[&str]) -> Result<String> {
    Ok(String::from_utf8_lossy(&git_bytes(cwd, args)?)
        .trim_end()
        .to_string())
}

/// Multi-valued config read; empty when unset (git exits 1 for that).
pub fn git_config_all(cwd: &Path, key: &str) -> Result<Vec<String>> {
    let out = Command::new("git")
        .args(["config", "--get-all", key])
        .current_dir(cwd)
        .output()
        .context("failed to spawn git")?;
    if !out.status.success() {
        return Ok(Vec::new());
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::to_string)
        .collect())
}

/// Run git, ignoring failure (for idempotent cleanup like --unset-all).
pub fn git_ignore_failure(cwd: &Path, args: &[&str]) {
    let _ = Command::new("git").args(args).current_dir(cwd).output();
}

pub fn repo_root() -> Result<PathBuf> {
    let cwd = std::env::current_dir()?;
    let root = git_string(&cwd, &["rev-parse", "--show-toplevel"])
        .context("not inside a git repository")?;
    Ok(PathBuf::from(root))
}

/// The absolute `.git` directory — where per-clone gitwasm state (the verdict
/// cache) lives, alongside git's own. Handles worktrees and `$GIT_DIR`.
pub fn git_dir(cwd: &Path) -> Result<PathBuf> {
    let dir = git_string(cwd, &["rev-parse", "--absolute-git-dir"])
        .context("locating the .git directory")?;
    Ok(PathBuf::from(dir))
}
