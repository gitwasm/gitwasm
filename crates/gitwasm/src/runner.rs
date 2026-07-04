use crate::manifest::Limits;
use anyhow::{Context, Result};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use wasmtime::{Config, Engine, Linker, Module, Store, StoreLimits, StoreLimitsBuilder};
use wasmtime_wasi::p1::{add_to_linker_sync, WasiP1Ctx};
use wasmtime_wasi::p2::pipe::MemoryOutputPipe;
use wasmtime_wasi::{DirPerms, FilePerms, I32Exit, WasiCtxBuilder};

const EPOCH_TICK_MS: u64 = 100;
const OUTPUT_CAP_BYTES: usize = 1024 * 1024;

/// The sandbox a module runs in. This is the whole security story:
/// the module sees exactly one directory (mounted at "."), its argv,
/// and captured stdout/stderr. No network, no env, no other files —
/// and fuel, memory, and wall-clock limits bound what it may consume.
pub struct Sandbox<'a> {
    pub dir: &'a Path,
    pub writable: bool,
    pub argv: Vec<String>,
    pub limits: Limits,
}

struct Ctx {
    wasi: WasiP1Ctx,
    limits: StoreLimits,
}

/// Run a WASI command module to completion; returns its exit code.
pub fn run_module(wasm_path: &Path, sandbox: Sandbox) -> Result<i32> {
    let module_bytes = std::fs::read(wasm_path)
        .with_context(|| format!("reading wasm module {}", wasm_path.display()))?;
    run_module_bytes(&module_bytes, sandbox)
        .with_context(|| format!("running {}", wasm_path.display()))
}

pub fn run_module_bytes(wasm: &[u8], sandbox: Sandbox) -> Result<i32> {
    let mut config = Config::new();
    config.consume_fuel(true);
    config.epoch_interruption(true);
    let engine = Engine::new(&config)?;
    let module = Module::new(&engine, wasm).context("compiling wasm module")?;

    let mut linker: Linker<Ctx> = Linker::new(&engine);
    add_to_linker_sync(&mut linker, |ctx| &mut ctx.wasi)?;

    let (dir_perms, file_perms) = if sandbox.writable {
        (DirPerms::all(), FilePerms::all())
    } else {
        (DirPerms::READ, FilePerms::READ)
    };

    // Module output is captured, sanitized, and re-emitted — untrusted code
    // must not be able to write raw escape sequences to the user's terminal.
    let stdout_pipe = MemoryOutputPipe::new(OUTPUT_CAP_BYTES);
    let stderr_pipe = MemoryOutputPipe::new(OUTPUT_CAP_BYTES);

    let mut builder = WasiCtxBuilder::new();
    builder
        .stdout(stdout_pipe.clone())
        .stderr(stderr_pipe.clone())
        .args(&sandbox.argv)
        .preopened_dir(sandbox.dir, ".", dir_perms, file_perms)
        .with_context(|| format!("preopening {}", sandbox.dir.display()))?;

    let ctx = Ctx {
        wasi: builder.build_p1(),
        limits: StoreLimitsBuilder::new()
            .memory_size(sandbox.limits.memory_bytes as usize)
            .build(),
    };
    let mut store = Store::new(&engine, ctx);
    store.limiter(|ctx| &mut ctx.limits);
    store.set_fuel(sandbox.limits.fuel)?;
    store.set_epoch_deadline(sandbox.limits.wall_ms.div_ceil(EPOCH_TICK_MS).max(1));

    // Wall-clock ticker: catches modules stalled where fuel isn't consumed.
    let stop = Arc::new(AtomicBool::new(false));
    let ticker = {
        let engine = engine.clone();
        let stop = Arc::clone(&stop);
        std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                std::thread::sleep(std::time::Duration::from_millis(EPOCH_TICK_MS));
                engine.increment_epoch();
            }
        })
    };

    let outcome = (|| {
        let instance = linker
            .instantiate(&mut store, &module)
            .context("instantiating module")?;
        let start = instance
            .get_typed_func::<(), ()>(&mut store, "_start")
            .context("module has no _start (not a WASI command module?)")?;
        match start.call(&mut store, ()) {
            Ok(()) => Ok(0),
            Err(err) => match err.downcast_ref::<I32Exit>() {
                Some(exit) => Ok(exit.0),
                None => Err(err.context("module trapped (limit exceeded or crash)")),
            },
        }
    })();

    stop.store(true, Ordering::Relaxed);
    let _ = ticker.join();
    drop(store); // release the pipes' write ends before reading contents

    print!("{}", sanitize(&stdout_pipe.contents()));
    eprint!("{}", sanitize(&stderr_pipe.contents()));
    outcome
}

/// Strip control bytes (except newline and tab) so untrusted module output
/// cannot inject terminal escape sequences.
fn sanitize(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .chars()
        .filter(|&c| c == '\n' || c == '\t' || !c.is_control())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tiny_limits() -> Limits {
        Limits {
            fuel: 1_000_000,
            memory_bytes: 64 * 1024 * 1024,
            wall_ms: 60_000,
        }
    }

    /// A hostile module that loops forever must be stopped by the fuel limit,
    /// not hang the host.
    #[test]
    fn fuel_limit_stops_infinite_loop() {
        let wasm = wat::parse_str(r#"(module (func (export "_start") (loop br 0)))"#).unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let result = run_module_bytes(
            &wasm,
            Sandbox {
                dir: tmp.path(),
                writable: false,
                argv: vec!["loop".into()],
                limits: tiny_limits(),
            },
        );
        let err = result.expect_err("infinite loop must trap on fuel exhaustion");
        assert!(
            format!("{err:#}").contains("fuel"),
            "unexpected error: {err:#}"
        );
    }

    /// With effectively unlimited fuel, the wall-clock deadline must fire.
    #[test]
    fn wall_clock_deadline_stops_infinite_loop() {
        let wasm = wat::parse_str(r#"(module (func (export "_start") (loop br 0)))"#).unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let result = run_module_bytes(
            &wasm,
            Sandbox {
                dir: tmp.path(),
                writable: false,
                argv: vec!["loop".into()],
                limits: Limits {
                    fuel: u64::MAX,
                    memory_bytes: 64 * 1024 * 1024,
                    wall_ms: 300,
                },
            },
        );
        let err = result.expect_err("infinite loop must trap on epoch deadline");
        // wasmtime reports epoch-deadline expiry as a plain "interrupt" trap.
        assert!(
            format!("{err:#}").contains("interrupt"),
            "unexpected error: {err:#}"
        );
    }

    #[test]
    fn sanitize_strips_escapes() {
        assert_eq!(
            sanitize(b"ok\n\x1b[2J\x1b]0;pwned\x07line\ttab\n"),
            "ok\n[2J]0;pwnedline\ttab\n"
        );
    }
}
