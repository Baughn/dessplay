//! The fuzz crates are workspaces of their own (excluded from the root),
//! so the root's `[patch.crates-io]` does not reach them: each mirrors
//! it. A patch added at the root and not mirrored broke both fuzz crates
//! unnoticed, since nothing in the gate builds them (found 2026-10-13:
//! the vendored crdts' `iter_entries` was missing there).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// `name -> canonical path` for every entry of the manifest's
/// `[patch.crates-io]` section. Only the `name = { path = "..." }` form
/// is understood; anything else in the section fails the test loudly.
fn patches(manifest: &Path) -> BTreeMap<String, PathBuf> {
    let text = std::fs::read_to_string(manifest)
        .unwrap_or_else(|e| panic!("reading {}: {e}", manifest.display()));
    let dir = manifest.parent().expect("manifest has a directory");
    let mut patches = BTreeMap::new();
    let mut in_section = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            in_section = line == "[patch.crates-io]";
            continue;
        }
        if !in_section || line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (name, value) = line
            .split_once('=')
            .unwrap_or_else(|| panic!("{}: unparsed patch line {line:?}", manifest.display()));
        let path = value
            .split_once("path = \"")
            .and_then(|(_, rest)| rest.split_once('"'))
            .map(|(path, _)| path)
            .unwrap_or_else(|| panic!("{}: patch without a path: {line:?}", manifest.display()));
        let resolved = dir
            .join(path)
            .canonicalize()
            .unwrap_or_else(|e| panic!("{}: patch path {path:?}: {e}", manifest.display()));
        patches.insert(name.trim().to_string(), resolved);
    }
    patches
}

#[test]
fn fuzz_crates_mirror_the_root_patches() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("dessplay-core sits in the workspace root");
    let expected = patches(&root.join("Cargo.toml"));
    assert!(!expected.is_empty(), "the root has no patches to mirror");
    for fuzz in ["dessplay-core/fuzz", "dessplay/fuzz"] {
        let manifest = root.join(fuzz).join("Cargo.toml");
        assert_eq!(
            patches(&manifest),
            expected,
            "{fuzz}/Cargo.toml's [patch.crates-io] must mirror the root's"
        );
    }
}
