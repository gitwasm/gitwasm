//! Stock modules embedded in the host binary at build time (see build.rs),
//! so `gitwasm init` can scaffold a repo with zero downloads.

pub struct StockModule {
    /// File name written into `.gitwasm/`.
    pub file: &'static str,
    pub bytes: &'static [u8],
    /// Hook name this module registers for, if any.
    pub hook: Option<&'static str>,
    /// Merge patterns this module registers for.
    pub merge_patterns: &'static [&'static str],
    /// Whether `gitwasm init` enables it by default (disabled ones are
    /// still written to `.gitwasm/` with a commented-out manifest entry).
    pub default_on: bool,
    pub summary: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitProfile {
    All,
    Lockfiles,
    Hooks,
}

impl InitProfile {
    pub fn name(self) -> &'static str {
        match self {
            InitProfile::All => "all",
            InitProfile::Lockfiles => "lockfiles",
            InitProfile::Hooks => "hooks",
        }
    }
}

pub const STOCK: &[StockModule] = &[
    StockModule {
        file: "lockfile-merge.wasm",
        bytes: include_bytes!(concat!(env!("OUT_DIR"), "/lockfile-merge.wasm")),
        hook: None,
        merge_patterns: &["package-lock.json", "package.json"],
        default_on: true,
        summary: "structural 3-way merge for JSON (package-lock.json, package.json)",
    },
    StockModule {
        file: "cargo-lock-merge.wasm",
        bytes: include_bytes!(concat!(env!("OUT_DIR"), "/cargo-lock-merge.wasm")),
        hook: None,
        merge_patterns: &["Cargo.lock"],
        default_on: true,
        summary: "structural 3-way merge for Cargo.lock",
    },
    StockModule {
        file: "lineset-merge.wasm",
        bytes: include_bytes!(concat!(env!("OUT_DIR"), "/lineset-merge.wasm")),
        hook: None,
        merge_patterns: &["go.sum"],
        default_on: true,
        summary: "set-algebra 3-way merge for line-set files (go.sum)",
    },
    StockModule {
        file: "yarn-lock-merge.wasm",
        bytes: include_bytes!(concat!(env!("OUT_DIR"), "/yarn-lock-merge.wasm")),
        hook: None,
        merge_patterns: &["yarn.lock"],
        default_on: true,
        summary: "structural 3-way merge for yarn.lock v1",
    },
    StockModule {
        file: "poetry-lock-merge.wasm",
        bytes: include_bytes!(concat!(env!("OUT_DIR"), "/poetry-lock-merge.wasm")),
        hook: None,
        merge_patterns: &["poetry.lock"],
        default_on: true,
        summary: "structural 3-way merge for poetry.lock",
    },
    StockModule {
        file: "pnpm-lock-merge.wasm",
        bytes: include_bytes!(concat!(env!("OUT_DIR"), "/pnpm-lock-merge.wasm")),
        hook: None,
        merge_patterns: &["pnpm-lock.yaml"],
        default_on: true,
        summary: "structural 3-way merge for pnpm-lock.yaml",
    },
    StockModule {
        file: "secret-scan.wasm",
        bytes: include_bytes!(concat!(env!("OUT_DIR"), "/secret-scan.wasm")),
        hook: Some("pre-commit"),
        merge_patterns: &[],
        default_on: true,
        summary: "block commits containing credentials",
    },
    StockModule {
        file: "commit-lint.wasm",
        bytes: include_bytes!(concat!(env!("OUT_DIR"), "/commit-lint.wasm")),
        hook: Some("commit-msg"),
        merge_patterns: &[],
        default_on: false,
        summary: "enforce conventional commit messages (opt-in)",
    },
];

fn module_in_profile(module: &StockModule, profile: InitProfile) -> bool {
    match profile {
        InitProfile::All => true,
        InitProfile::Lockfiles => !module.merge_patterns.is_empty(),
        InitProfile::Hooks => module.hook.is_some(),
    }
}

fn module_enabled_in_manifest(module: &StockModule, profile: InitProfile) -> bool {
    match profile {
        InitProfile::All | InitProfile::Hooks => module.default_on,
        InitProfile::Lockfiles => !module.merge_patterns.is_empty(),
    }
}

pub fn modules_for(profile: InitProfile) -> Vec<&'static StockModule> {
    STOCK
        .iter()
        .filter(|module| module_in_profile(module, profile))
        .collect()
}

/// Render the default manifest.toml for `gitwasm init`.
pub fn default_manifest_for(profile: InitProfile) -> String {
    let mut hooks = String::new();
    let mut merges = String::new();
    for module in modules_for(profile) {
        if let Some(hook) = module.hook {
            let prefix = if module_enabled_in_manifest(module, profile) {
                ""
            } else {
                "# "
            };
            hooks.push_str(&format!("{prefix}{hook} = \"{}\"\n", module.file));
        }
        for pattern in module.merge_patterns {
            merges.push_str(&format!(
                "\n[[merge]]\npattern = \"{pattern}\"\nmodule = \"{}\"\n",
                module.file
            ));
        }
    }
    format!(
        "# gitwasm manifest — maps git extension points to sandboxed wasm modules\n\
         # stored in this directory. Profile: {}\n\
         # See https://github.com/gitwasm/gitwasm\n\
         \n[hooks]\n{hooks}{merges}",
        profile.name()
    )
}

/// Render the default manifest.toml for `gitwasm init`.
#[allow(dead_code)]
pub fn default_manifest() -> String {
    default_manifest_for(InitProfile::All)
}

/// The .gitattributes lines the default manifest needs. The `-text` line is
/// load-bearing: git EOL conversion would silently change file hashes across
/// platforms and break signature verification.
pub fn gitattributes_lines_for(profile: InitProfile) -> Vec<String> {
    let mut lines = vec![".gitwasm/** -text".to_string()];
    lines.extend(
        modules_for(profile)
            .into_iter()
            .flat_map(|module| module.merge_patterns.iter())
            .map(|pattern| format!("{pattern} merge=gitwasm")),
    );
    lines
}

#[allow(dead_code)]
pub fn gitattributes_lines() -> Vec<String> {
    gitattributes_lines_for(InitProfile::All)
}

#[cfg(test)]
mod tests {
    use super::{default_manifest_for, gitattributes_lines_for, modules_for, InitProfile, STOCK};

    #[test]
    fn lockfiles_profile_contains_merge_modules_but_no_hooks() {
        let files: Vec<&str> = modules_for(InitProfile::Lockfiles)
            .into_iter()
            .map(|module| module.file)
            .collect();

        assert!(files.contains(&"lockfile-merge.wasm"));
        assert!(files.contains(&"cargo-lock-merge.wasm"));
        assert!(files.contains(&"yarn-lock-merge.wasm"));
        assert!(files.contains(&"poetry-lock-merge.wasm"));
        assert!(files.contains(&"lineset-merge.wasm"));
        assert!(!files.contains(&"secret-scan.wasm"));
        assert!(!files.contains(&"commit-lint.wasm"));

        let manifest = default_manifest_for(InitProfile::Lockfiles);
        assert!(manifest.contains("pattern = \"package-lock.json\""));
        assert!(manifest.contains("pattern = \"go.sum\""));
        assert!(!manifest.contains("pre-commit"));
        assert!(!manifest.contains("commit-msg"));
    }

    #[test]
    fn hooks_profile_contains_hooks_but_no_merge_rules() {
        let files: Vec<&str> = modules_for(InitProfile::Hooks)
            .into_iter()
            .map(|module| module.file)
            .collect();

        assert!(files.contains(&"secret-scan.wasm"));
        assert!(files.contains(&"commit-lint.wasm"));
        assert!(!files.contains(&"lockfile-merge.wasm"));
        assert!(!files.contains(&"cargo-lock-merge.wasm"));

        let manifest = default_manifest_for(InitProfile::Hooks);
        assert!(manifest.contains("pre-commit = \"secret-scan.wasm\""));
        assert!(manifest.contains("# commit-msg = \"commit-lint.wasm\""));
        assert!(!manifest.contains("[[merge]]"));
    }

    #[test]
    fn lockfiles_gitattributes_do_not_enable_hooks() {
        let lines = gitattributes_lines_for(InitProfile::Lockfiles);

        assert_eq!(lines[0], ".gitwasm/** -text");
        assert!(lines.contains(&"package-lock.json merge=gitwasm".to_string()));
        assert!(lines.contains(&"go.sum merge=gitwasm".to_string()));
        assert!(!lines.iter().any(|line| line.contains("secret-scan")));
    }

    #[test]
    fn pnpm_is_registered_as_stock_lockfile_merge_driver() {
        let pnpm = STOCK
            .iter()
            .find(|module| module.file == "pnpm-lock-merge.wasm")
            .expect("pnpm stock module is registered");

        assert_eq!(pnpm.merge_patterns, &["pnpm-lock.yaml"]);
        assert!(pnpm.default_on);

        let attrs = gitattributes_lines_for(InitProfile::Lockfiles);
        assert!(attrs.contains(&"pnpm-lock.yaml merge=gitwasm".to_string()));
    }
}
