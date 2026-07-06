//! Compiles the stock modules and drops the blobs into OUT_DIR so the host
//! binary can embed them (see src/stock.rs). This is what lets `gitwasm init`
//! scaffold a repo offline from a single binary.
//!
//! There are two module ABIs, built differently:
//!
//! * **preview1 modules** are WASI command modules (a `main`) built to
//!   `wasm32-wasip1` and embedded as-is.
//! * **component modules** export the typed `gitwasm:merge/driver` world
//!   (`wit/driver.wit`). They are built to `wasm32-unknown-unknown` — so the
//!   result imports nothing but its own WIT world — and then wrapped into a
//!   WASI 0.2 component here with `wit_component`.

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

const PREVIEW1_MODULES: &[&str] = &[
    "lockfile-merge",
    "cargo-lock-merge",
    "yarn-lock-merge",
    "poetry-lock-merge",
    "pnpm-lock-merge",
    "secret-scan",
    "commit-lint",
];

const COMPONENT_MODULES: &[&str] = &["lineset-merge"];

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let workspace_root = manifest_dir.parent().unwrap().parent().unwrap();
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());

    for module in PREVIEW1_MODULES.iter().chain(COMPONENT_MODULES) {
        println!(
            "cargo::rerun-if-changed={}",
            workspace_root.join("modules").join(module).display()
        );
    }
    // Component modules embed the WIT type section at compile time, so a WIT
    // change must rebuild them.
    println!(
        "cargo::rerun-if-changed={}",
        workspace_root.join("wit").display()
    );

    let cargo = env::var("CARGO").unwrap_or_else(|_| "cargo".into());

    // 1) preview1 command modules -> wasm32-wasip1, embedded verbatim.
    let p1_target = out_dir.join("modules-target-wasip1");
    build_modules(
        &cargo,
        workspace_root,
        &p1_target,
        "wasm32-wasip1",
        PREVIEW1_MODULES,
    );
    let p1_release = p1_target.join("wasm32-wasip1").join("release");
    for module in PREVIEW1_MODULES {
        let blob = p1_release.join(format!("{module}.wasm"));
        assert!(blob.exists(), "expected module blob at {}", blob.display());
        write_if_changed(&out_dir.join(format!("{module}.wasm")), &read(&blob));
    }

    // 2) component modules -> wasm32-unknown-unknown, then componentized.
    let cm_target = out_dir.join("modules-target-component");
    build_modules(
        &cargo,
        workspace_root,
        &cm_target,
        "wasm32-unknown-unknown",
        COMPONENT_MODULES,
    );
    let cm_release = cm_target.join("wasm32-unknown-unknown").join("release");
    for module in COMPONENT_MODULES {
        // cdylib output uses the lib name (hyphens become underscores).
        let core = cm_release.join(format!("{}.wasm", module.replace('-', "_")));
        assert!(core.exists(), "expected core module at {}", core.display());
        let component = wit_component::ComponentEncoder::default()
            .module(&read(&core))
            .unwrap_or_else(|e| panic!("feeding {module} core module to encoder: {e}"))
            .validate(true)
            .encode()
            .unwrap_or_else(|e| panic!("encoding {module} component: {e}"));
        write_if_changed(&out_dir.join(format!("{module}.wasm")), &component);
    }
}

fn build_modules(
    cargo: &str,
    workspace_root: &Path,
    target_dir: &Path,
    target: &str,
    modules: &[&str],
) {
    let mut cmd = Command::new(cargo);
    cmd.arg("build")
        .arg("--release")
        .arg("--target")
        .arg(target)
        // A private target dir avoids deadlocking on the parent build's lock.
        .arg("--target-dir")
        .arg(target_dir)
        .arg("--manifest-path")
        .arg(workspace_root.join("Cargo.toml"));
    for module in modules {
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
            "building stock wasm modules for {target} failed. Are the targets \
             installed? Try: rustup target add wasm32-wasip1 wasm32-unknown-unknown"
        );
    }
}

fn read(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()))
}

fn write_if_changed(dest: &Path, bytes: &[u8]) {
    if std::fs::read(dest).ok().as_deref() != Some(bytes) {
        std::fs::write(dest, bytes).unwrap();
    }
}
