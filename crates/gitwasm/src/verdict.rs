//! Verdicts: content-addressed, re-derivable records of what a module computed.
//!
//! Every gitwasm module run is a pure function of content-addressed inputs — the
//! sandbox gives it no ambient state, and a component module imports nothing at
//! all. So each run can be recorded as a *verdict*: a small signed-able fact of
//! the form "module M applied to inputs I under engine E yields output O". Those
//! facts are
//!
//! * **cacheable** — an identical `(module, inputs)` replays the stored result
//!   instead of re-executing;
//! * **re-derivable** — because everything needed to reproduce the run is stored
//!   content-addressed, `gitwasm audit` can re-run it and confirm the record is
//!   honest. You cannot lie about a verdict.
//!
//! The store lives under `<git-dir>/gitwasm/` (a per-clone cache today; making
//! verdicts travel between clones through a git ref is the next step). Blobs and
//! module bytes are deduplicated by their sha256, so a verdict is just metadata
//! plus references.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

/// Bumping any of these — the domain tag, the record shape, or the runtime —
/// changes verdict keys, so stale verdicts simply stop matching (fail-safe).
const KEY_DOMAIN: &str = "gitwasm-verdict-v1";
pub const ENGINE_ID: &str = concat!("gitwasm-", env!("CARGO_PKG_VERSION"), "+wasmtime-38");

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// The three sides of a merge, referenced by the sha256 of their bytes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergeInputs {
    pub base: String,
    pub ours: String,
    pub theirs: String,
}

/// A recorded merge computation. Field order matters: the `[inputs]` table is
/// serialized last so the TOML stays valid (scalars before tables).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Verdict {
    pub key: String,
    pub kind: String,
    /// sha256 of the module wasm (also stored as a blob, so audit is self-contained).
    pub module: String,
    /// Repo-relative path being merged (part of the module's input).
    pub path: String,
    pub exit_code: i32,
    /// sha256 of the merged bytes; absent when the module reported a conflict.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub result: Option<String>,
    pub engine: String,
    pub inputs: MergeInputs,
}

/// Derive the content address of a merge computation. Any change to the module,
/// any of the three sides, the path, or the engine yields a different key — so a
/// hit is only ever a genuinely identical computation.
pub fn merge_key(module: &str, inputs: &MergeInputs, path: &str) -> String {
    let mut h = Sha256::new();
    for part in [
        KEY_DOMAIN,
        "merge",
        module,
        &inputs.base,
        &inputs.ours,
        &inputs.theirs,
        path,
        ENGINE_ID,
    ] {
        h.update(part.as_bytes());
        h.update([0]);
    }
    hex::encode(h.finalize())
}

/// A content-addressed store of verdicts and the blobs they reference.
pub struct Store {
    root: PathBuf,
}

impl Store {
    /// Open (creating if needed) the store under `<git-dir>/gitwasm/`.
    pub fn open(git_dir: &Path) -> Result<Store> {
        let root = git_dir.join("gitwasm");
        fs::create_dir_all(root.join("blobs")).context("creating verdict blob store")?;
        fs::create_dir_all(root.join("verdicts")).context("creating verdict store")?;
        Ok(Store { root })
    }

    fn blob_path(&self, hash: &str) -> PathBuf {
        self.root.join("blobs").join(hash)
    }

    fn verdict_path(&self, key: &str) -> PathBuf {
        self.root.join("verdicts").join(format!("{key}.toml"))
    }

    /// Store bytes by content address (idempotent); returns their sha256.
    pub fn put_blob(&self, bytes: &[u8]) -> Result<String> {
        let hash = sha256_hex(bytes);
        let path = self.blob_path(&hash);
        if !path.exists() {
            fs::write(&path, bytes).with_context(|| format!("writing blob {hash}"))?;
        }
        Ok(hash)
    }

    /// Fetch a blob, verifying it still hashes to its own address — this is what
    /// makes a replay trustworthy even if the cache directory was tampered with.
    pub fn get_blob(&self, hash: &str) -> Result<Vec<u8>> {
        let bytes =
            fs::read(self.blob_path(hash)).with_context(|| format!("reading blob {hash}"))?;
        if sha256_hex(&bytes) != hash {
            bail!("blob {hash} is corrupted (content does not match its address)");
        }
        Ok(bytes)
    }

    pub fn get(&self, key: &str) -> Result<Option<Verdict>> {
        let path = self.verdict_path(key);
        if !path.exists() {
            return Ok(None);
        }
        let text = fs::read_to_string(&path)?;
        Ok(Some(toml::from_str(&text).with_context(|| {
            format!("parsing verdict {}", path.display())
        })?))
    }

    pub fn put(&self, verdict: &Verdict) -> Result<()> {
        let text = toml::to_string(verdict).context("serializing verdict")?;
        fs::write(self.verdict_path(&verdict.key), text)
            .with_context(|| format!("writing verdict {}", verdict.key))
    }

    /// Every recorded verdict, in stable key order.
    pub fn list(&self) -> Result<Vec<Verdict>> {
        let mut out = Vec::new();
        let dir = self.root.join("verdicts");
        let mut entries: Vec<PathBuf> = fs::read_dir(&dir)?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|e| e == "toml"))
            .collect();
        entries.sort();
        for path in entries {
            out.push(toml::from_str(&fs::read_to_string(&path)?)?);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs(base: &[u8], ours: &[u8], theirs: &[u8]) -> MergeInputs {
        MergeInputs {
            base: sha256_hex(base),
            ours: sha256_hex(ours),
            theirs: sha256_hex(theirs),
        }
    }

    #[test]
    fn key_is_stable_and_input_sensitive() {
        let m = sha256_hex(b"module");
        let a = merge_key(&m, &inputs(b"b", b"o", b"t"), "go.sum");
        let same = merge_key(&m, &inputs(b"b", b"o", b"t"), "go.sum");
        let diff_side = merge_key(&m, &inputs(b"b", b"o", b"T"), "go.sum");
        let diff_path = merge_key(&m, &inputs(b"b", b"o", b"t"), "other");
        assert_eq!(a, same, "identical computation must key identically");
        assert_ne!(a, diff_side, "a changed side must change the key");
        assert_ne!(a, diff_path, "a changed path must change the key");
    }

    #[test]
    fn store_roundtrip_and_blob_integrity() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(tmp.path()).unwrap();

        let blob = store.put_blob(b"hello").unwrap();
        assert_eq!(store.get_blob(&blob).unwrap(), b"hello");

        let verdict = Verdict {
            key: "abc".into(),
            kind: "merge".into(),
            module: sha256_hex(b"module"),
            path: "go.sum".into(),
            exit_code: 0,
            result: Some(blob.clone()),
            engine: ENGINE_ID.into(),
            inputs: inputs(b"b", b"o", b"t"),
        };
        store.put(&verdict).unwrap();
        let loaded = store.get("abc").unwrap().expect("verdict present");
        assert_eq!(loaded.result.as_deref(), Some(blob.as_str()));
        assert_eq!(loaded.inputs.theirs, sha256_hex(b"t"));
        assert_eq!(store.list().unwrap().len(), 1);

        // Corrupting a blob must be caught on read, not silently replayed.
        fs::write(store.blob_path(&blob), b"tampered").unwrap();
        assert!(store.get_blob(&blob).is_err(), "tampered blob must fail");
    }

    #[test]
    fn get_missing_verdict_is_none() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(tmp.path()).unwrap();
        assert!(store.get("nope").unwrap().is_none());
    }
}
