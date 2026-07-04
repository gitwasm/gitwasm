//! Structural 3-way merge for JSON lockfiles (package-lock.json and friends).
//!
//! Git's line-based merge conflicts whenever two branches add adjacent lines —
//! which is exactly what happens every time two branches each add a dependency.
//! Structurally those edits are disjoint keys in a map: a trivial clean merge.
//!
//! Invoked by the gitwasm host as: lockfile-merge <base> <ours> <theirs> <result> [path]
//! inside a sandbox whose only visible directory contains those files.

use serde_json::{Map, Value};
use std::process::exit;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 5 {
        eprintln!("usage: lockfile-merge <base> <ours> <theirs> <result> [path]");
        exit(2);
    }
    let base = read_json(&args[1]);
    let ours = read_json(&args[2]);
    let theirs = read_json(&args[3]);

    let mut conflicts = Vec::new();
    let merged = merge3(
        base.as_ref(),
        ours.as_ref(),
        theirs.as_ref(),
        "$",
        &mut conflicts,
    );

    if !conflicts.is_empty() {
        for c in &conflicts {
            eprintln!("lockfile-merge: real conflict at {c}");
        }
        exit(1);
    }

    let out = merged.unwrap_or(Value::Null);
    let text = serde_json::to_string_pretty(&out).expect("serialize merged JSON");
    std::fs::write(&args[4], text + "\n").expect("write result");
    eprintln!("lockfile-merge: clean structural merge");
}

fn read_json(path: &str) -> Option<Value> {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    if text.trim().is_empty() {
        return None; // absent side (e.g. empty %O when there is no common base)
    }
    match serde_json::from_str(&text) {
        Ok(v) => Some(v),
        Err(err) => {
            eprintln!("lockfile-merge: {path} is not valid JSON ({err}) — refusing");
            exit(1);
        }
    }
}

/// Classic 3-way merge over JSON values; `None` means "absent on this side".
/// Returns the merged value, recording paths of true conflicts.
fn merge3(
    base: Option<&Value>,
    ours: Option<&Value>,
    theirs: Option<&Value>,
    path: &str,
    conflicts: &mut Vec<String>,
) -> Option<Value> {
    if ours == theirs {
        return ours.cloned(); // same change (or same deletion) on both sides
    }
    if ours == base {
        return theirs.cloned(); // only theirs changed
    }
    if theirs == base {
        return ours.cloned(); // only ours changed
    }

    // Both sides changed, differently.
    match (ours, theirs) {
        (Some(Value::Object(o)), Some(Value::Object(t))) => {
            let empty = Map::new();
            let b = match base {
                Some(Value::Object(m)) => m,
                _ => &empty,
            };
            // Key order matters to the tools that own these files: npm keeps
            // dependency maps alphabetized, but e.g. package.json's top level
            // is deliberately unsorted. Mirror whatever the inputs do — if all
            // sides are sorted, insert additions in sorted position; otherwise
            // preserve base order and append each side's additions in its own
            // order.
            let mut keys: Vec<&String> = Vec::new();
            for key in b.keys().chain(o.keys()).chain(t.keys()) {
                if !keys.contains(&key) {
                    keys.push(key);
                }
            }
            let all_sorted = [b, o, t].iter().all(|m| m.keys().is_sorted());
            if all_sorted {
                keys.sort();
            }
            let mut out = Map::new();
            for key in keys {
                let child_path = format!("{path}.{key}");
                if let Some(v) = merge3(b.get(key), o.get(key), t.get(key), &child_path, conflicts)
                {
                    out.insert(key.clone(), v);
                }
            }
            Some(Value::Object(out))
        }
        (Some(Value::String(o)), Some(Value::String(t)))
            if parse_version(o).is_some() && parse_version(t).is_some() =>
        {
            // Both bumped the same version string: take the higher one.
            let winner = if parse_version(o) >= parse_version(t) {
                o
            } else {
                t
            };
            eprintln!("lockfile-merge: {path}: both sides bumped, taking {winner}");
            Some(Value::String(winner.clone()))
        }
        _ => {
            conflicts.push(path.to_string());
            ours.cloned()
        }
    }
}

/// Lenient dotted-numeric version ("1.2.3", "10.0"); None if it isn't one.
fn parse_version(s: &str) -> Option<Vec<u64>> {
    let parts: Vec<&str> = s.trim_start_matches(['^', '~', 'v']).split('.').collect();
    if parts.is_empty() || parts.len() > 4 {
        return None;
    }
    parts.iter().map(|p| p.parse::<u64>().ok()).collect()
}

#[cfg(test)]
mod tests {
    use super::merge3;
    use serde_json::{json, Value};

    fn run(base: Value, ours: Value, theirs: Value) -> Result<Value, Vec<String>> {
        let mut conflicts = Vec::new();
        let merged = merge3(Some(&base), Some(&ours), Some(&theirs), "$", &mut conflicts);
        if conflicts.is_empty() {
            Ok(merged.unwrap_or(Value::Null))
        } else {
            Err(conflicts)
        }
    }

    #[test]
    fn disjoint_additions_merge_clean() {
        let base = json!({"deps": {"a": "1.0.0"}});
        let ours = json!({"deps": {"a": "1.0.0", "b": "2.0.0"}});
        let theirs = json!({"deps": {"a": "1.0.0", "c": "3.0.0"}});
        let merged = run(base, ours, theirs).unwrap();
        assert_eq!(merged["deps"]["b"], "2.0.0");
        assert_eq!(merged["deps"]["c"], "3.0.0");
    }

    #[test]
    fn concurrent_version_bumps_take_higher() {
        let base = json!({"a": "1.0.0"});
        let ours = json!({"a": "1.2.0"});
        let theirs = json!({"a": "1.10.0"});
        assert_eq!(run(base, ours, theirs).unwrap()["a"], "1.10.0");
    }

    #[test]
    fn deletion_vs_modification_conflicts() {
        let base = json!({"a": {"x": 1}});
        let ours = json!({});
        let theirs = json!({"a": {"x": 2}});
        let conflicts = run(base, ours, theirs).unwrap_err();
        assert_eq!(conflicts, vec!["$.a"]);
    }

    #[test]
    fn agreed_deletion_is_clean() {
        let base = json!({"a": 1, "b": 2});
        let ours = json!({"b": 2});
        let theirs = json!({"b": 2});
        assert_eq!(run(base, ours, theirs).unwrap(), json!({"b": 2}));
    }

    #[test]
    fn sorted_maps_get_additions_in_sorted_position() {
        let base = json!({"a": 1, "m": 1, "z": 1});
        let ours = json!({"a": 1, "b": 1, "m": 1, "z": 1});
        let theirs = json!({"a": 1, "m": 1, "n": 1, "z": 1});
        let merged = run(base, ours, theirs).unwrap();
        let keys: Vec<&String> = merged.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["a", "b", "m", "n", "z"]);
    }

    #[test]
    fn unsorted_maps_keep_base_order_and_append() {
        let base = json!({"name": "x", "version": "1", "dependencies": {}});
        let ours = json!({"name": "x", "version": "1", "dependencies": {}, "scripts": {}});
        let theirs = json!({"name": "x", "version": "2", "dependencies": {}});
        let merged = run(base, ours, theirs).unwrap();
        let keys: Vec<&String> = merged.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["name", "version", "dependencies", "scripts"]);
    }

    #[test]
    fn non_version_scalar_conflict_is_reported() {
        let base = json!({"name": "app"});
        let ours = json!({"name": "app-one"});
        let theirs = json!({"name": "app-two"});
        assert_eq!(run(base, ours, theirs).unwrap_err(), vec!["$.name"]);
    }
}
