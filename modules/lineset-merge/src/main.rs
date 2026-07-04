//! 3-way merge for files that are semantically a *set of lines* — go.sum is
//! the canonical case: every line is an independent content-addressed fact,
//! so the correct merge is pure set algebra and can never conflict:
//! keep base minus what either side removed, plus what either side added.
//!
//! Invoked by the gitwasm host as: lineset-merge <base> <ours> <theirs> <result> [path]

use std::collections::BTreeSet;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 5 {
        eprintln!("usage: lineset-merge <base> <ours> <theirs> <result> [path]");
        std::process::exit(2);
    }
    let base = read_lines(&args[1]);
    let ours = read_lines(&args[2]);
    let theirs = read_lines(&args[3]);

    let merged: BTreeSet<&String> = base
        .iter()
        .filter(|line| ours.contains(*line) && theirs.contains(*line))
        .chain(ours.difference(&base))
        .chain(theirs.difference(&base))
        .collect();

    let mut out = String::new();
    for line in &merged {
        out.push_str(line);
        out.push('\n');
    }
    std::fs::write(&args[4], out).expect("write result");
    eprintln!(
        "lineset-merge: clean set merge ({} lines: {} base, {} ours, {} theirs)",
        merged.len(),
        base.len(),
        ours.len(),
        theirs.len()
    );
}

fn read_lines(path: &str) -> BTreeSet<String> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    fn set(lines: &[&str]) -> BTreeSet<String> {
        lines.iter().map(|s| s.to_string()).collect()
    }

    fn merge(base: &[&str], ours: &[&str], theirs: &[&str]) -> BTreeSet<String> {
        let (base, ours, theirs) = (set(base), set(ours), set(theirs));
        base.iter()
            .filter(|l| ours.contains(*l) && theirs.contains(*l))
            .chain(ours.difference(&base))
            .chain(theirs.difference(&base))
            .cloned()
            .collect()
    }

    #[test]
    fn additions_and_removals_from_both_sides() {
        let merged = merge(
            &["a v1", "b v1", "c v1"],
            &["a v1", "c v1", "d v1"], // removed b, added d
            &["a v1", "b v1", "e v1"], // removed c, added e
        );
        assert_eq!(merged, set(&["a v1", "d v1", "e v1"]));
    }
}
