//! Manifest signing: ed25519 signatures over the entire contents of
//! `.gitwasm/` (manifest, module blobs, hook shims — the shims matter most,
//! since git executes them natively).
//!
//! Trust model: `gitwasm install` pins the signing keys it sees into local
//! git config (trust-on-first-use — activation is already the explicit trust
//! decision, this makes it durable). From then on, every hook/merge run
//! verifies fail-closed: content not signed by a pinned key does not run.
//! `gitwasm trust` explicitly re-pins after a legitimate key rotation.

use anyhow::{bail, Context, Result};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

pub const SIGNATURES_FILE: &str = "signatures.toml";
const PAYLOAD_HEADER: &str = "gitwasm-signatures-v1\n";

#[derive(Debug, Deserialize)]
pub struct SignaturesFile {
    pub files: BTreeMap<String, String>,
    #[serde(default)]
    pub signatures: Vec<SignatureEntry>,
}

#[derive(Debug, Deserialize)]
pub struct SignatureEntry {
    pub key: String,
    pub sig: String,
}

pub enum VerifyOutcome {
    /// No signatures.toml present.
    Unsigned,
    /// Hashes match and these keys (hex) produced valid signatures.
    Valid(Vec<String>),
    /// Tampered, incomplete, or cryptographically invalid.
    Invalid(String),
}

/// Hash every file under `.gitwasm/` except signatures.toml itself.
/// Names are forward-slash relative paths; hashes are lowercase sha256 hex.
pub fn collect_files(gitwasm_dir: &Path) -> Result<BTreeMap<String, String>> {
    let mut files = BTreeMap::new();
    walk(gitwasm_dir, gitwasm_dir, &mut files)?;
    files.remove(SIGNATURES_FILE);
    Ok(files)
}

fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, String>) -> Result<()> {
    for entry in fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let path = entry?.path();
        if path.is_dir() {
            walk(root, &path, out)?;
        } else {
            let name = path
                .strip_prefix(root)
                .expect("walk stays under root")
                .to_string_lossy()
                .replace('\\', "/");
            let bytes = fs::read(&path)?;
            out.insert(name, hex::encode(Sha256::digest(&bytes)));
        }
    }
    Ok(())
}

/// The canonical byte string that gets signed.
pub fn payload(files: &BTreeMap<String, String>) -> Vec<u8> {
    let mut out = PAYLOAD_HEADER.as_bytes().to_vec();
    for (name, hash) in files {
        out.extend_from_slice(name.as_bytes());
        out.push(b'\n');
        out.extend_from_slice(hash.as_bytes());
        out.push(b'\n');
    }
    out
}

/// Full verification of a `.gitwasm/` directory against its signatures.toml.
pub fn verify_dir(gitwasm_dir: &Path) -> Result<VerifyOutcome> {
    let sig_path = gitwasm_dir.join(SIGNATURES_FILE);
    if !sig_path.exists() {
        return Ok(VerifyOutcome::Unsigned);
    }
    let recorded: SignaturesFile =
        toml::from_str(&fs::read_to_string(&sig_path)?).context("parsing signatures.toml")?;
    let actual = collect_files(gitwasm_dir)?;

    if actual != recorded.files {
        let mut differences = Vec::new();
        for name in actual.keys().chain(recorded.files.keys()) {
            if actual.get(name) != recorded.files.get(name) && !differences.contains(name) {
                differences.push(name.clone());
            }
        }
        return Ok(VerifyOutcome::Invalid(format!(
            "content does not match signed hashes: {}",
            differences.join(", ")
        )));
    }

    let message = payload(&recorded.files);
    let mut valid_keys = Vec::new();
    for entry in &recorded.signatures {
        let Ok(key_bytes) = hex::decode(&entry.key) else {
            continue;
        };
        let Ok(key_array) = <[u8; 32]>::try_from(key_bytes.as_slice()) else {
            continue;
        };
        let Ok(key) = VerifyingKey::from_bytes(&key_array) else {
            continue;
        };
        let Ok(sig_bytes) = hex::decode(&entry.sig) else {
            continue;
        };
        let Ok(sig_array) = <[u8; 64]>::try_from(sig_bytes.as_slice()) else {
            continue;
        };
        if key
            .verify(&message, &Signature::from_bytes(&sig_array))
            .is_ok()
        {
            valid_keys.push(entry.key.clone());
        }
    }
    if valid_keys.is_empty() {
        return Ok(VerifyOutcome::Invalid(
            "signatures.toml present but contains no valid signature".into(),
        ));
    }
    Ok(VerifyOutcome::Valid(valid_keys))
}

/// Render signatures.toml for the given files, signed by `key`.
pub fn render_signatures(files: &BTreeMap<String, String>, key: &SigningKey) -> String {
    let signature: Signature = key.sign(&payload(files));
    let mut out = String::from(
        "# Written by `gitwasm sign` — hashes of every file in .gitwasm/ plus\n\
         # ed25519 signatures over them. Verified fail-closed on every run.\n\n[files]\n",
    );
    for (name, hash) in files {
        out.push_str(&format!("\"{name}\" = \"{hash}\"\n"));
    }
    out.push_str(&format!(
        "\n[[signatures]]\nkey = \"{}\"\nsig = \"{}\"\n",
        hex::encode(key.verifying_key().to_bytes()),
        hex::encode(signature.to_bytes()),
    ));
    out
}

/// Private key location: GITWASM_KEY_PATH, or ~/.gitwasm/key.
pub fn key_path() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("GITWASM_KEY_PATH") {
        return Ok(PathBuf::from(path));
    }
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .context("cannot locate home directory (set GITWASM_KEY_PATH)")?;
    Ok(PathBuf::from(home).join(".gitwasm").join("key"))
}

pub fn generate_key(path: &Path) -> Result<SigningKey> {
    if path.exists() {
        bail!(
            "key already exists at {} — refusing to overwrite",
            path.display()
        );
    }
    let mut seed = [0u8; 32];
    getrandom::getrandom(&mut seed).context("gathering entropy")?;
    let key = SigningKey::from_bytes(&seed);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, hex::encode(seed))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(key)
}

pub fn load_key() -> Result<SigningKey> {
    let path = key_path()?;
    let text = fs::read_to_string(&path).with_context(|| {
        format!(
            "no signing key at {} — run `gitwasm keygen` first",
            path.display()
        )
    })?;
    let bytes = hex::decode(text.trim()).context("key file is not valid hex")?;
    let seed: [u8; 32] = bytes
        .as_slice()
        .try_into()
        .map_err(|_| anyhow::anyhow!("key file must be a 32-byte hex seed"))?;
    Ok(SigningKey::from_bytes(&seed))
}

pub fn fingerprint(key_hex: &str) -> &str {
    &key_hex[..key_hex.len().min(16)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_verify_tamper_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("manifest.toml"), "[hooks]\n").unwrap();
        fs::create_dir_all(dir.path().join("hooks")).unwrap();
        fs::write(dir.path().join("hooks/pre-commit"), "#!/bin/sh\n").unwrap();

        let key = SigningKey::from_bytes(&[7u8; 32]);
        let files = collect_files(dir.path()).unwrap();
        assert_eq!(files.len(), 2, "shims must be covered by signing");
        fs::write(
            dir.path().join(SIGNATURES_FILE),
            render_signatures(&files, &key),
        )
        .unwrap();

        let VerifyOutcome::Valid(keys) = verify_dir(dir.path()).unwrap() else {
            panic!("fresh signature must verify");
        };
        assert_eq!(keys, vec![hex::encode(key.verifying_key().to_bytes())]);

        // Tamper with the hook shim — exactly the attack that matters.
        fs::write(
            dir.path().join("hooks/pre-commit"),
            "#!/bin/sh\ncurl evil\n",
        )
        .unwrap();
        let VerifyOutcome::Invalid(reason) = verify_dir(dir.path()).unwrap() else {
            panic!("tampered shim must fail verification");
        };
        assert!(reason.contains("hooks/pre-commit"), "reason: {reason}");
    }
}
