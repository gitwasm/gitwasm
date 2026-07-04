mod commands;
mod gitutil;
mod manifest;
mod runner;
mod signing;
mod stock;

use std::process::ExitCode;

const USAGE: &str = "\
gitwasm — repo-embedded, sandboxed git behavior via WebAssembly

USAGE:
  gitwasm init                             scaffold .gitwasm/ with stock modules + activate
  gitwasm install                          activate .gitwasm/ modules in this clone
  gitwasm list                             show what this repo's manifest activates
  gitwasm keygen                           generate a maintainer signing key (stored in your home)
  gitwasm sign                             sign .gitwasm/ contents (writes signatures.toml)
  gitwasm verify                           check .gitwasm/ against its signatures
  gitwasm trust                            re-pin this clone's trust to the current signers
  gitwasm hook <name> [args...]            run the wasm hook registered for <name>
  gitwasm merge <base> <ours> <theirs> <path>
                                           run the wasm merge driver matching <path>
  gitwasm run <module.wasm> [args...]      run a module directly (preopens cwd, read-only)
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("init") => commands::init(),
        Some("install") => commands::install(),
        Some("list") => commands::list(),
        Some("keygen") => commands::keygen(),
        Some("sign") => commands::sign(),
        Some("verify") => commands::verify(),
        Some("trust") => commands::trust(),
        Some("hook") if args.len() >= 2 => commands::hook(&args[1], &args[2..]),
        Some("merge") if args.len() == 5 => commands::merge(&args[1], &args[2], &args[3], &args[4]),
        Some("run") if args.len() >= 2 => commands::run_direct(&args[1], &args[2..]),
        _ => {
            eprint!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(code) => ExitCode::from(code.clamp(0, 255) as u8),
        Err(err) => {
            eprintln!("gitwasm: error: {err:#}");
            ExitCode::from(2)
        }
    }
}
