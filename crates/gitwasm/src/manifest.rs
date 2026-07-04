use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const GITWASM_DIR: &str = ".gitwasm";
pub const MANIFEST_FILE: &str = "manifest.toml";

/// `.gitwasm/manifest.toml` — committed to the repo, maps git extension
/// points to wasm modules stored alongside it.
///
/// ```toml
/// [hooks]
/// pre-commit = "secret-scan.wasm"
///
/// [[merge]]
/// pattern = "package-lock.json"
/// module = "lockfile-merge.wasm"
/// ```
#[derive(Debug, Default, Deserialize)]
pub struct Manifest {
    #[serde(default)]
    pub hooks: BTreeMap<String, String>,
    #[serde(default)]
    pub merge: Vec<MergeRule>,
    #[serde(default)]
    pub limits: Limits,
}

#[derive(Debug, Deserialize)]
pub struct MergeRule {
    pub pattern: String,
    pub module: String,
}

/// Resource caps applied to every module run. Deliberately generous
/// defaults — they exist to stop hostile or broken modules, not to
/// tune performance.
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct Limits {
    #[serde(default = "default_fuel")]
    pub fuel: u64,
    #[serde(default = "default_memory")]
    pub memory_bytes: u64,
    #[serde(default = "default_wall_ms")]
    pub wall_ms: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            fuel: default_fuel(),
            memory_bytes: default_memory(),
            wall_ms: default_wall_ms(),
        }
    }
}

fn default_fuel() -> u64 {
    10_000_000_000 // ~10B instructions: seconds of CPU, far above any honest module
}

fn default_memory() -> u64 {
    512 * 1024 * 1024
}

fn default_wall_ms() -> u64 {
    60_000 // catches what fuel can't: modules stalled in blocking syscalls
}

impl Manifest {
    /// Load from the repo root. A repo without a manifest is valid (empty manifest).
    pub fn load(repo_root: &Path) -> Result<Manifest> {
        let path = repo_root.join(GITWASM_DIR).join(MANIFEST_FILE);
        if !path.exists() {
            return Ok(Manifest::default());
        }
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))
    }

    pub fn module_path(repo_root: &Path, module: &str) -> PathBuf {
        repo_root.join(GITWASM_DIR).join(module)
    }

    /// Find the merge module for a repo-relative path (gitattributes-style:
    /// a pattern without `/` matches the basename, otherwise the full path).
    pub fn merge_module(&self, path: &str) -> Option<&str> {
        let path = path.replace('\\', "/");
        let basename = path.rsplit('/').next().unwrap_or(&path);
        self.merge
            .iter()
            .find(|rule| {
                let subject = if rule.pattern.contains('/') {
                    path.as_str()
                } else {
                    basename
                };
                glob_match(&rule.pattern, subject)
            })
            .map(|rule| rule.module.as_str())
    }
}

/// Minimal glob: `*` matches any run of characters, everything else is literal.
fn glob_match(pattern: &str, subject: &str) -> bool {
    let (p, s): (Vec<char>, Vec<char>) = (pattern.chars().collect(), subject.chars().collect());
    let (mut pi, mut si) = (0usize, 0usize);
    let (mut star, mut mark) = (None::<usize>, 0usize);
    while si < s.len() {
        if pi < p.len() && (p[pi] == s[si]) {
            pi += 1;
            si += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = si;
            pi += 1;
        } else if let Some(sp) = star {
            pi = sp + 1;
            mark += 1;
            si = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

#[cfg(test)]
mod tests {
    use super::glob_match;

    #[test]
    fn globs() {
        assert!(glob_match("package-lock.json", "package-lock.json"));
        assert!(glob_match("*.lock", "Cargo.lock"));
        assert!(glob_match("locks/*.json", "locks/a.json"));
        assert!(!glob_match("*.lock", "Cargo.toml"));
    }
}
