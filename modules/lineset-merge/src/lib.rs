//! 3-way merge for files that are semantically a *set of lines* — go.sum is
//! the canonical case: every line is an independent content-addressed fact,
//! so the correct merge is pure set algebra and can never conflict:
//! keep base minus what either side removed, plus what either side added.
//!
//! Unlike gitwasm's preview1 modules, this ships as a WebAssembly **component**:
//! it exports the typed `gitwasm:merge/driver` world (see `wit/driver.wit`) and
//! imports *nothing at all*. The host passes the three sides as byte lists and
//! receives the merged bytes back across the typed boundary — no directory is
//! ever mounted into the sandbox. See SPEC.md §5.2.

use std::collections::BTreeSet;

/// Set-algebra 3-way merge: keep the base lines that survived on both sides,
/// plus every line either side added. Pure and total — a line-set merge can
/// never conflict, which is exactly why go.sum is a good fit. This is the whole
/// behavior; the component boundary below is a thin typed wrapper over it.
pub fn merge_lines(base: &[u8], ours: &[u8], theirs: &[u8]) -> Vec<u8> {
    let base = to_set(base);
    let ours = to_set(ours);
    let theirs = to_set(theirs);

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
    out.into_bytes()
}

fn to_set(bytes: &[u8]) -> BTreeSet<String> {
    String::from_utf8_lossy(bytes)
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(str::to_string)
        .collect()
}

// The component-model boundary. Compiled only for wasm32: the wit-bindgen
// exports name canonical-ABI symbols (`cabi_post_.../merge3`) that the host
// linker rejects, so on the host this module is just `merge_lines` above.
#[cfg(target_arch = "wasm32")]
mod component {
    wit_bindgen::generate!({
        world: "merge-driver",
        path: "../../wit",
    });

    struct Driver;

    impl exports::gitwasm::merge::driver::Guest for Driver {
        fn merge3(
            base: Vec<u8>,
            ours: Vec<u8>,
            theirs: Vec<u8>,
            _path: String,
        ) -> Result<Vec<u8>, String> {
            Ok(super::merge_lines(&base, &ours, &theirs))
        }
    }

    export!(Driver);
}

#[cfg(test)]
mod tests {
    use super::merge_lines;

    #[test]
    fn additions_and_removals_from_both_sides() {
        let merged = merge_lines(
            b"a v1\nb v1\nc v1\n",
            b"a v1\nc v1\nd v1\n", // removed b, added d
            b"a v1\nb v1\ne v1\n", // removed c, added e
        );
        assert_eq!(merged, b"a v1\nd v1\ne v1\n");
    }

    #[test]
    fn identical_sides_are_unchanged() {
        let x = b"golang.org/x/sys v0.1.0 h1:aaa=\ngolang.org/x/sys v0.1.0/go.mod h1:bbb=\n";
        assert_eq!(merge_lines(x, x, x), x);
    }

    #[test]
    fn output_is_sorted_and_deduplicated() {
        // go.sum is kept sorted; duplicate facts across sides collapse to one.
        let merged = merge_lines(b"", b"z\na\n", b"a\nm\n");
        assert_eq!(merged, b"a\nm\nz\n");
    }
}
