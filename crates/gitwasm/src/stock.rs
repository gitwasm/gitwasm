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

/// Render the default manifest.toml for `gitwasm init`.
pub fn default_manifest() -> String {
    let mut hooks = String::new();
    let mut merges = String::new();
    for module in STOCK {
        if let Some(hook) = module.hook {
            let prefix = if module.default_on { "" } else { "# " };
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
         # stored in this directory. See https://github.com/gitwasm/gitwasm\n\
         \n[hooks]\n{hooks}{merges}"
    )
}

/// The .gitattributes lines the default manifest needs. The `-text` line is
/// load-bearing: git EOL conversion would silently change file hashes across
/// platforms and break signature verification.
pub fn gitattributes_lines() -> Vec<String> {
    let mut lines = vec![".gitwasm/** -text".to_string()];
    lines.extend(
        STOCK
            .iter()
            .flat_map(|m| m.merge_patterns.iter())
            .map(|p| format!("{p} merge=gitwasm")),
    );
    lines
}
