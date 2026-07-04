//! Compiles the stock modules to wasm32-wasip1 and drops the blobs into
//! OUT_DIR so the host binary can embed them (see src/stock.rs). This is
//! what lets `gitwasm init` scaffold a repo offline from a single binary.

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

const STOCK_MODULES: &[&str] = &[
    "lockfile-merge",
    "cargo-lock-merge",
    "lineset-merge",
    "yarn-lock-merge",
    "poetry-lock-merge",
    "secret-scan",
    "commit-lint",
];

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let workspace_root = manifest_dir.parent().unwrap().parent().unwrap();
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    // A private target dir avoids deadlocking on the parent build's lock.
    let module_target = out_dir.join("modules-target");

    for module in STOCK_MODULES {
        println!(
            "cargo::rerun-if-changed={}",
            workspace_root.join("modules").join(module).display()
        );
    }

    let cargo = env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let mut cmd = Command::new(cargo);
    cmd.arg("build")
        .arg("--release")
        .arg("--target")
        .arg("wasm32-wasip1")
        .arg("--target-dir")
        .arg(&module_target)
        .arg("--manifest-path")
        .arg(workspace_root.join("Cargo.toml"));
    for module in STOCK_MODULES {
        cmd.arg("-p").arg(module);
    }
    // Host-specific flags must not leak into the wasm build.
    cmd.env_remove("RUSTFLAGS")
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env_remove("CARGO_TARGET_DIR");

    let status = cmd
        .status()
        .expect("failed to spawn cargo for module build");
    if !status.success() {
        panic!(
            "building stock wasm modules failed. \
             Is the target installed? Try: rustup target add wasm32-wasip1"
        );
    }

    let release = module_target.join("wasm32-wasip1").join("release");
    for module in STOCK_MODULES {
        let blob = release.join(format!("{module}.wasm"));
        assert!(blob.exists(), "expected module blob at {}", blob.display());
        copy_if_changed(&blob, &out_dir.join(format!("{module}.wasm")));
    }
}

fn copy_if_changed(src: &Path, dest: &Path) {
    let src_bytes = std::fs::read(src).unwrap();
    if std::fs::read(dest).ok().as_deref() != Some(&src_bytes) {
        std::fs::write(dest, src_bytes).unwrap();
    }
}
