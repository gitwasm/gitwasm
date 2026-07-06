//! Structural 3-way merge for pnpm-lock.yaml.
//!
//! Invoked by the gitwasm host as: pnpm-lock-merge <base> <ours> <theirs> <result> [path]

use serde_yaml::{Mapping, Value};
use std::process::exit;

const DEPENDENCY_GROUPS: &[&str] = &[
    "dependencies",
    "devDependencies",
    "optionalDependencies",
    "peerDependencies",
];

#[derive(Clone, Debug, PartialEq)]
struct Lock {
    doc: Mapping,
}

#[derive(Debug)]
struct CommandError {
    code: i32,
    message: String,
}

impl CommandError {
    fn new(code: i32, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    fn diagnostic(&self) -> String {
        format!("pnpm-lock-merge: {}", self.message)
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if let Err(err) = run(&args) {
        eprintln!("{}", err.diagnostic());
        exit(err.code);
    }
}

fn run(args: &[String]) -> Result<(), CommandError> {
    if args.len() < 5 {
        return Err(CommandError::new(
            2,
            "usage: pnpm-lock-merge <base> <ours> <theirs> <result> [path]",
        ));
    }
    let base = load_lock(&args[1])?;
    let ours = load_lock(&args[2])?;
    let theirs = load_lock(&args[3])?;

    match merge3(&base, &ours, &theirs) {
        Ok((merged, notes)) => {
            for note in notes {
                eprintln!("pnpm-lock-merge: {note}");
            }
            write_result(&args[4], &merged)?;
            eprintln!("pnpm-lock-merge: clean structural merge");
            Ok(())
        }
        Err(err) => Err(CommandError::new(1, err)),
    }
}

fn load_lock(path: &str) -> Result<Lock, CommandError> {
    let text = std::fs::read_to_string(path)
        .map_err(|err| CommandError::new(1, format!("{path}: cannot read ({err}) -- refusing")))?;
    parse(&text).map_err(|err| CommandError::new(1, format!("{path}: {err} -- refusing")))
}

fn write_result(path: &str, lock: &Lock) -> Result<(), CommandError> {
    std::fs::write(path, render(lock))
        .map_err(|err| CommandError::new(1, format!("{path}: cannot write ({err}) -- refusing")))
}

fn parse(text: &str) -> Result<Lock, String> {
    if text.trim().is_empty() {
        return Ok(Lock {
            doc: Mapping::new(),
        });
    }

    let value: Value =
        serde_yaml::from_str(text).map_err(|err| format!("not valid YAML ({err})"))?;
    let Value::Mapping(doc) = value else {
        return Err("expected top-level YAML mapping".to_string());
    };

    if !doc.contains_key(Value::String("lockfileVersion".to_string())) {
        return Err("missing required lockfileVersion".to_string());
    }

    validate_schema(&doc)?;

    Ok(Lock { doc })
}

fn validate_schema(doc: &Mapping) -> Result<(), String> {
    if let Some(importers) = get_key(doc, "importers") {
        let importers_path = vec!["importers".to_string()];
        let importers = require_mapping(importers, &importers_path)?;
        for (importer, value) in importers {
            let importer_path = child_path(&importers_path, importer);
            let importer = require_mapping(value, &importer_path)?;
            for group in DEPENDENCY_GROUPS {
                if let Some(records) = get_key(importer, group) {
                    let group_path = child_path(&importer_path, &Value::String((*group).into()));
                    let records = require_mapping(records, &group_path)?;
                    for (dependency, record) in records {
                        require_mapping(record, &child_path(&group_path, dependency))?;
                    }
                }
            }
        }
    }

    validate_record_map(doc, "packages")?;
    validate_record_map(doc, "snapshots")?;
    Ok(())
}

fn validate_record_map(doc: &Mapping, section: &str) -> Result<(), String> {
    if let Some(records) = get_key(doc, section) {
        let section_path = vec![section.to_string()];
        let records = require_mapping(records, &section_path)?;
        for (key, record) in records {
            require_mapping(record, &child_path(&section_path, key))?;
        }
    }
    Ok(())
}

fn get_key<'a>(mapping: &'a Mapping, key: &str) -> Option<&'a Value> {
    mapping.get(Value::String(key.to_string()))
}

fn require_mapping<'a>(value: &'a Value, path: &[String]) -> Result<&'a Mapping, String> {
    match value {
        Value::Mapping(mapping) => Ok(mapping),
        _ => Err(format!("{} must be a mapping", path_display(path))),
    }
}

fn render(lock: &Lock) -> String {
    let mut text =
        serde_yaml::to_string(&Value::Mapping(lock.doc.clone())).expect("serialize pnpm lockfile");
    if !text.ends_with('\n') {
        text.push('\n');
    }
    text
}

fn merge3(base: &Lock, ours: &Lock, theirs: &Lock) -> Result<(Lock, Vec<String>), String> {
    Ok((
        Lock {
            doc: merge_mapping(Some(&base.doc), &ours.doc, &theirs.doc, &[])?,
        },
        Vec::new(),
    ))
}

fn merge_mapping(
    base: Option<&Mapping>,
    ours: &Mapping,
    theirs: &Mapping,
    path: &[String],
) -> Result<Mapping, String> {
    let empty = Mapping::new();
    let base = base.unwrap_or(&empty);

    let mut keys: Vec<Value> = base
        .keys()
        .chain(ours.keys())
        .chain(theirs.keys())
        .cloned()
        .collect();
    keys.sort_by_key(key_name);
    keys.dedup();

    let mut out = Mapping::new();
    for key in keys {
        let child_path = child_path(path, &key);
        if let Some(value) = merge_value(
            base.get(&key),
            ours.get(&key),
            theirs.get(&key),
            &child_path,
        )? {
            out.insert(key, value);
        }
    }

    Ok(out)
}

fn merge_value(
    base: Option<&Value>,
    ours: Option<&Value>,
    theirs: Option<&Value>,
    path: &[String],
) -> Result<Option<Value>, String> {
    if ours == theirs {
        return Ok(ours.cloned());
    }
    if ours == base {
        return Ok(theirs.cloned());
    }
    if theirs == base {
        return Ok(ours.cloned());
    }

    match (base, ours, theirs) {
        (Some(Value::Mapping(base)), Some(Value::Mapping(ours)), Some(Value::Mapping(theirs))) => {
            if is_atomic_record_path(path) {
                return Err(format!("real conflict at {}", path_display(path)));
            }
            Ok(Some(Value::Mapping(merge_mapping(
                Some(base),
                ours,
                theirs,
                path,
            )?)))
        }
        (None, Some(Value::Mapping(ours)), Some(Value::Mapping(theirs))) => {
            if is_atomic_record_path(path) {
                return Err(format!("real conflict at {}", path_display(path)));
            }
            Ok(Some(Value::Mapping(merge_mapping(
                None, ours, theirs, path,
            )?)))
        }
        _ => Err(format!("real conflict at {}", path_display(path))),
    }
}

fn key_name(key: &Value) -> String {
    match key {
        Value::String(value) => value.clone(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::Null => "null".to_string(),
        _ => serde_yaml::to_string(key)
            .unwrap_or_else(|_| format!("{key:?}"))
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" "),
    }
}

fn child_path(parent: &[String], key: &Value) -> Vec<String> {
    let mut path = parent.to_vec();
    path.push(key_name(key));
    path
}

fn path_display(path: &[String]) -> String {
    path.join(".")
}

fn is_atomic_record_path(path: &[String]) -> bool {
    match path {
        [root, _] if root == "packages" || root == "snapshots" => true,
        [root, _, group, _] if root == "importers" && is_dependency_group(group) => true,
        _ => false,
    }
}

fn is_dependency_group(group: &str) -> bool {
    DEPENDENCY_GROUPS.contains(&group)
}

#[cfg(test)]
mod tests {
    use super::{load_lock, merge3, parse, render, write_result, Lock};
    use serde_yaml::Value;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    const BASE: &str = r#"
lockfileVersion: '9.0'
settings:
  autoInstallPeers: true
importers:
  .:
    dependencies:
      lodash:
        specifier: ^4.17.21
        version: 4.17.21
packages:
  lodash@4.17.21:
    resolution:
      integrity: sha512-lodash
snapshots:
  lodash@4.17.21: {}
"#;

    fn lock(text: &str) -> Lock {
        parse(text).unwrap()
    }

    fn temp_path(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "pnpm-lock-merge-{}-{unique}-{name}",
            std::process::id()
        ))
    }

    #[test]
    fn disjoint_dependency_additions_merge_cleanly() {
        let ours = r#"
lockfileVersion: '9.0'
settings:
  autoInstallPeers: true
importers:
  .:
    dependencies:
      left-pad:
        specifier: ^1.3.0
        version: 1.3.0
      lodash:
        specifier: ^4.17.21
        version: 4.17.21
packages:
  left-pad@1.3.0:
    resolution:
      integrity: sha512-left
  lodash@4.17.21:
    resolution:
      integrity: sha512-lodash
snapshots:
  left-pad@1.3.0: {}
  lodash@4.17.21: {}
"#;
        let theirs = r#"
lockfileVersion: '9.0'
settings:
  autoInstallPeers: true
importers:
  .:
    dependencies:
      lodash:
        specifier: ^4.17.21
        version: 4.17.21
      right-pad:
        specifier: ^1.0.1
        version: 1.0.1
packages:
  lodash@4.17.21:
    resolution:
      integrity: sha512-lodash
  right-pad@1.0.1:
    resolution:
      integrity: sha512-right
snapshots:
  lodash@4.17.21: {}
  right-pad@1.0.1: {}
"#;

        let (merged, notes) = merge3(&lock(BASE), &lock(ours), &lock(theirs)).unwrap();

        assert!(notes.is_empty());
        assert_eq!(
            merged.doc["importers"]["."]["dependencies"]["left-pad"]["version"].as_str(),
            Some("1.3.0")
        );
        assert_eq!(
            merged.doc["importers"]["."]["dependencies"]["right-pad"]["version"].as_str(),
            Some("1.0.1")
        );
        assert!(merged.doc["packages"]["left-pad@1.3.0"].is_mapping());
        assert!(merged.doc["packages"]["right-pad@1.0.1"].is_mapping());
        assert!(merged.doc["snapshots"]["left-pad@1.3.0"].is_mapping());
        assert!(merged.doc["snapshots"]["right-pad@1.0.1"].is_mapping());

        let rendered = render(&merged);
        assert!(rendered.ends_with('\n'));
        parse(&rendered).unwrap();
    }

    #[test]
    fn same_package_changed_differently_conflicts() {
        let base = r#"
lockfileVersion: '9.0'
packages:
  left-pad@1.3.0:
    resolution:
      integrity: sha512-base
"#;
        let ours = r#"
lockfileVersion: '9.0'
packages:
  left-pad@1.3.0:
    resolution:
      integrity: sha512-ours
"#;
        let theirs = r#"
lockfileVersion: '9.0'
packages:
  left-pad@1.3.0:
    resolution:
      integrity: sha512-theirs
"#;

        let err = merge3(&lock(base), &lock(ours), &lock(theirs)).unwrap_err();

        assert!(err.contains("packages.left-pad@1.3.0"));
    }

    #[test]
    fn dependency_record_changed_differently_conflicts() {
        let base = r#"
lockfileVersion: '9.0'
importers:
  .:
    dependencies:
      left-pad:
        specifier: ^1.3.0
        version: 1.3.0
"#;
        let ours = r#"
lockfileVersion: '9.0'
importers:
  .:
    dependencies:
      left-pad:
        specifier: ~1.3.0
        version: 1.3.0
"#;
        let theirs = r#"
lockfileVersion: '9.0'
importers:
  .:
    dependencies:
      left-pad:
        specifier: ^1.3.0
        version: 1.3.1
"#;

        let err = merge3(&lock(base), &lock(ours), &lock(theirs)).unwrap_err();

        assert!(err.contains("dependencies"));
        assert!(err.contains("left-pad"));
    }

    #[test]
    fn package_record_changed_differently_conflicts() {
        let base = r#"
lockfileVersion: '9.0'
packages:
  left-pad@1.3.0:
    resolution:
      integrity: sha512-base
"#;
        let ours = r#"
lockfileVersion: '9.0'
packages:
  left-pad@1.3.0:
    resolution:
      integrity: sha512-ours
"#;
        let theirs = r#"
lockfileVersion: '9.0'
packages:
  left-pad@1.3.0:
    resolution:
      integrity: sha512-base
    dependencies:
      foo: 1.0.0
"#;

        let err = merge3(&lock(base), &lock(ours), &lock(theirs)).unwrap_err();

        assert!(err.contains("packages.left-pad@1.3.0"));
    }

    #[test]
    fn snapshot_record_changed_differently_conflicts() {
        let base = r#"
lockfileVersion: '9.0'
snapshots:
  left-pad@1.3.0:
    dependencies:
      foo: 1.0.0
"#;
        let ours = r#"
lockfileVersion: '9.0'
snapshots:
  left-pad@1.3.0:
    dependencies:
      foo: 1.0.1
"#;
        let theirs = r#"
lockfileVersion: '9.0'
snapshots:
  left-pad@1.3.0:
    dependencies:
      foo: 1.0.0
    optional: true
"#;

        let err = merge3(&lock(base), &lock(ours), &lock(theirs)).unwrap_err();

        assert!(err.contains("snapshots.left-pad@1.3.0"));
    }

    #[test]
    fn malformed_yaml_is_refused() {
        let err = parse("lockfileVersion: '9.0'\npackages:\n  left-pad@1.3.0: [").unwrap_err();

        assert!(err.contains("valid YAML"));
    }

    #[test]
    fn unsupported_top_level_shape_is_refused() {
        let err = parse("- lockfileVersion: '9.0'\n").unwrap_err();

        assert!(err.contains("top-level YAML mapping"));
    }

    #[test]
    fn missing_lockfile_version_is_refused() {
        let err = parse("packages:\n  left-pad@1.3.0: {}\n").unwrap_err();

        assert!(err.contains("lockfileVersion"));
    }

    #[test]
    fn malformed_structural_sections_are_refused() {
        let cases = [
            (
                "lockfileVersion: '9.0'\nimporters: []\n",
                "importers",
            ),
            (
                "lockfileVersion: '9.0'\nimporters:\n  .: []\n",
                "importers",
            ),
            (
                "lockfileVersion: '9.0'\nimporters:\n  .:\n    dependencies: []\n",
                "importers...dependencies",
            ),
            (
                "lockfileVersion: '9.0'\nimporters:\n  .:\n    dependencies:\n      left-pad: 1.3.0\n",
                "importers...dependencies.left-pad",
            ),
            (
                "lockfileVersion: '9.0'\npackages: []\n",
                "packages",
            ),
            (
                "lockfileVersion: '9.0'\npackages:\n  left-pad@1.3.0: []\n",
                "packages.left-pad@1.3.0",
            ),
            (
                "lockfileVersion: '9.0'\nsnapshots: []\n",
                "snapshots",
            ),
            (
                "lockfileVersion: '9.0'\nsnapshots:\n  left-pad@1.3.0: []\n",
                "snapshots.left-pad@1.3.0",
            ),
        ];

        for (text, path) in cases {
            let err = parse(text).unwrap_err();

            assert!(err.contains(path), "expected {err:?} to identify {path:?}");
            assert!(err.contains("mapping"));
        }
    }

    #[test]
    fn missing_input_file_is_refused() {
        let path = temp_path("missing-input.yaml");
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir_all(&path);

        let err = load_lock(path.to_str().unwrap()).unwrap_err();

        assert_eq!(err.code, 1);
        assert!(err.diagnostic().starts_with("pnpm-lock-merge: "));
        assert!(err.diagnostic().contains("cannot read"));
        assert!(err.diagnostic().contains("-- refusing"));
    }

    #[test]
    fn write_result_errors_are_reported() {
        let path = temp_path("write-target");
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir(&path).unwrap();

        let err =
            write_result(path.to_str().unwrap(), &lock("lockfileVersion: '9.0'\n")).unwrap_err();

        std::fs::remove_dir(&path).unwrap();
        assert_eq!(err.code, 1);
        assert!(err.diagnostic().starts_with("pnpm-lock-merge: "));
        assert!(err.diagnostic().contains("cannot write"));
        assert!(err.diagnostic().contains("-- refusing"));
    }

    #[test]
    fn empty_input_parses_as_absent_side() {
        let lock = parse("").unwrap();

        assert!(lock.doc.is_empty());
    }

    #[test]
    fn delete_vs_modify_conflicts() {
        let base = r#"
lockfileVersion: '9.0'
packages:
  left-pad@1.3.0:
    resolution:
      integrity: sha512-base
"#;
        let ours = r#"
lockfileVersion: '9.0'
packages: {}
"#;
        let theirs = r#"
lockfileVersion: '9.0'
packages:
  left-pad@1.3.0:
    resolution:
      integrity: sha512-base
    dependencies:
      foo: 1.0.0
"#;

        let err = merge3(&lock(base), &lock(ours), &lock(theirs)).unwrap_err();

        assert_eq!(err, "real conflict at packages.left-pad@1.3.0");
    }

    #[test]
    fn one_sided_change_wins() {
        let base = lock(BASE);
        let ours = lock(BASE);
        let theirs = lock(
            r#"
lockfileVersion: '9.0'
settings:
  autoInstallPeers: false
importers:
  .:
    dependencies:
      lodash:
        specifier: ^4.17.21
        version: 4.17.21
packages:
  lodash@4.17.21:
    resolution:
      integrity: sha512-lodash
snapshots:
  lodash@4.17.21: {}
"#,
        );

        let (merged, notes) = merge3(&base, &ours, &theirs).unwrap();

        assert!(notes.is_empty());
        assert_eq!(
            merged.doc["settings"]["autoInstallPeers"],
            Value::Bool(false)
        );
    }
}
