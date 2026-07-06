mod commands;
mod gitutil;
mod manifest;
mod runner;
mod signing;
mod stock;
mod verdict;

use std::process::ExitCode;

const USAGE: &str = "\
gitwasm — repo-embedded, sandboxed git behavior via WebAssembly

USAGE:
  gitwasm init [all|lockfiles|hooks]       scaffold .gitwasm/ stock modules + activate
  gitwasm install                          activate .gitwasm/ modules in this clone
  gitwasm list                             show what this repo's manifest activates
  gitwasm keygen                           generate a maintainer signing key (stored in your home)
  gitwasm sign                             sign .gitwasm/ contents (writes signatures.toml)
  gitwasm verify                           check .gitwasm/ against its signatures
  gitwasm trust                            re-pin this clone's trust to the current signers
  gitwasm verdicts                         list the content-addressed verdicts cached in this clone
  gitwasm audit [key]                      re-derive cached verdicts and confirm they reproduce
  gitwasm hook <name> [args...]            run the wasm hook registered for <name>
  gitwasm merge <base> <ours> <theirs> <path>
                                           run the wasm merge driver matching <path>
  gitwasm run <module.wasm> [args...]      run a module directly (preopens cwd, read-only)
";

/// Restore the default SIGPIPE disposition. Rust's runtime installs SIG_IGN,
/// which turns a closed reader (`gitwasm list | head`) into an EPIPE that makes
/// the print machinery panic; the Unix-native behavior is to die quietly on the
/// signal instead. No-op on platforms without SIGPIPE.
#[cfg(unix)]
fn reset_sigpipe() {
    // SAFETY: called once at startup, before any other thread exists.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
}
#[cfg(not(unix))]
fn reset_sigpipe() {}

fn init_profile_arg(args: &[String]) -> Option<Result<Option<&str>, ()>> {
    match args.first().map(String::as_str) {
        Some("init") => match args.len() {
            1 => Some(Ok(None)),
            2 => Some(Ok(Some(args[1].as_str()))),
            _ => Some(Err(())),
        },
        _ => None,
    }
}

fn main() -> ExitCode {
    reset_sigpipe();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = if let Some(profile_arg) = init_profile_arg(&args) {
        match profile_arg {
            Ok(profile_arg) => commands::init(profile_arg),
            Err(()) => {
                eprint!("{USAGE}");
                return ExitCode::from(2);
            }
        }
    } else {
        match args.first().map(String::as_str) {
            Some("install") => commands::install(),
            Some("list") => commands::list(),
            Some("keygen") => commands::keygen(),
            Some("sign") => commands::sign(),
            Some("verify") => commands::verify(),
            Some("trust") => commands::trust(),
            Some("verdicts") => commands::verdicts(),
            Some("audit") => commands::audit(args.get(1).map(String::as_str)),
            Some("hook") if args.len() >= 2 => commands::hook(&args[1], &args[2..]),
            Some("merge") if args.len() == 5 => {
                commands::merge(&args[1], &args[2], &args[3], &args[4])
            }
            Some("run") if args.len() >= 2 => commands::run_direct(&args[1], &args[2..]),
            _ => {
                eprint!("{USAGE}");
                return ExitCode::from(2);
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn init_profile_arg_defaults_when_profile_omitted() {
        assert_eq!(init_profile_arg(&args(&["init"])), Some(Ok(None)));
    }

    #[test]
    fn init_profile_arg_accepts_one_profile() {
        assert_eq!(
            init_profile_arg(&args(&["init", "lockfiles"])),
            Some(Ok(Some("lockfiles")))
        );
    }

    #[test]
    fn init_profile_arg_rejects_surplus_args() {
        assert_eq!(
            init_profile_arg(&args(&["init", "lockfiles", "extra"])),
            Some(Err(()))
        );
    }

    #[test]
    fn init_profile_arg_ignores_other_commands() {
        assert_eq!(init_profile_arg(&args(&["list"])), None);
    }
}
