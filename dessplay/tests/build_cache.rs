//! Real Cargo builds prove that pruning keeps every warm-build input.
#![cfg(unix)]

use proptest::prelude::*;
use std::{fs, path::Path, process::Command, time::SystemTime};

fn old_file(path: &Path, contents: &[u8]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
    fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1))
        .unwrap();
}

fn write_messages(path: &Path, messages: &[serde_json::Value]) {
    fs::write(
        path,
        messages
            .iter()
            .map(|m| format!("{m}\n"))
            .collect::<String>(),
    )
    .unwrap();
}

proptest! {
    #[test]
    fn retention_depends_on_build_membership_not_artifact_age(units in prop::collection::btree_map(any::<u64>(), any::<bool>(), 1..12)) {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("target");
        let release = target.join("release");
        let mut messages = Vec::new();
        let mut expectations = Vec::new();
        for (i, (hash, keep)) in units.into_iter().enumerate() {
            let keep = keep || i == 0;
            let library = release.join(format!("deps/libpkg-{hash:016x}.rlib"));
            let fingerprint = release.join(format!(".fingerprint/pkg-{hash:016x}/lib-pkg"));
            old_file(&library, b"cached library");
            old_file(&fingerprint, b"0000000000000000");
            old_file(&fingerprint.with_extension("json"), b"{\"deps\":[]}");
            if keep {
                messages.push(serde_json::json!({"reason":"compiler-artifact", "filenames":[library], "target":{"name":"pkg"}, "executable":null}));
            }
            expectations.push((library, fingerprint, keep));
        }
        messages.push(serde_json::json!({"reason":"build-finished", "success":true}));
        let manifest = temp.path().join("messages.json");
        write_messages(&manifest, &messages);
        dessplay::build_cache::prune(&manifest, &target, SystemTime::now() + std::time::Duration::from_secs(1)).unwrap();
        for (library, fingerprint, keep) in expectations {
            prop_assert_eq!(library.exists(), keep);
            prop_assert_eq!(fingerprint.exists(), keep);
        }
    }
}

#[test]
fn incomplete_build_cannot_authorize_deletion() {
    let temp = tempfile::tempdir().unwrap();
    let target = temp.path().join("target");
    let old = target.join("release/deps/libpkg-0000000000000001.rlib");
    old_file(&old, b"valuable cache");
    let manifest = temp.path().join("messages.json");
    for messages in [
        "",
        "{",
        "{\"reason\":\"build-finished\",\"success\":false}\n",
        "{\"reason\":\"build-finished\",\"success\":true}\n",
    ] {
        fs::write(&manifest, messages).unwrap();
        assert!(dessplay::build_cache::prune(&manifest, &target, SystemTime::now()).is_err());
        assert!(old.exists());
    }
}

fn build(root: &Path, obsolete: bool, tracking: bool) -> Vec<serde_json::Value> {
    let mut command = Command::new(env!("CARGO"));
    command
        .args(["build", "--offline", "--release", "--message-format=json"])
        .current_dir(root)
        .env("CARGO_INCREMENTAL", "1")
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env_remove("RUSTC_WORKSPACE_WRAPPER")
        .env_remove("RUSTC_WRAPPER")
        .env_remove("DESSPLAY_ORIGINAL_RUSTC_WRAPPER")
        .env_remove("CARGO_BUILD_BUILD_DIR")
        .env("CARGO_TARGET_DIR", root.join("target"))
        .env(
            "RUSTFLAGS",
            if obsolete { "-Cmetadata=obsolete" } else { "" },
        );
    let wrapper = Path::new(env!("CARGO_MANIFEST_DIR")).join("../build-cache-rustc.sh");
    let original = root.join("existing-wrapper.sh");
    if tracking {
        command
            .env("RUSTC_WRAPPER", wrapper)
            .env("DESSPLAY_ORIGINAL_RUSTC_WRAPPER", &original);
    } else {
        command.env("RUSTC_WRAPPER", &original);
    }
    command.env("WRAPPER_LOG", root.join("wrapper.log"));
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let messages: Vec<serde_json::Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .filter(|line| line.starts_with('{'))
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    fs::write(
        root.join("messages.json"),
        messages
            .iter()
            .map(|m| format!("{m}\n"))
            .collect::<String>(),
    )
    .unwrap();
    messages
}

#[test]
fn obsolete_builds_are_removed_and_current_build_stays_fully_fresh() {
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("cache with ' quotes");
    fs::create_dir(&directory).unwrap();
    let root = directory.as_path();
    use std::os::unix::fs::PermissionsExt;
    let original = root.join("existing-wrapper.sh");
    fs::write(
        &original,
        "#!/bin/sh\nprintf x >> \"$WRAPPER_LOG\"\nexec \"$@\"\n",
    )
    .unwrap();
    fs::set_permissions(&original, fs::Permissions::from_mode(0o755)).unwrap();
    fs::create_dir(root.join("src")).unwrap();
    fs::create_dir_all(root.join("helper/src")).unwrap();
    fs::write(
        root.join("Cargo.toml"),
        r#"[package]
name = "cache-probe"
version = "0.1.0"
edition = "2024"
[dependencies]
helper = { path = "helper" }
[profile.release]
strip = false
[profile.release.build-override]
strip = false
"#,
    )
    .unwrap();
    fs::write(
        root.join("helper/Cargo.toml"),
        "[package]\nname='helper'\nversion='0.1.0'\nedition='2024'\n",
    )
    .unwrap();
    fs::write(
        root.join("helper/src/lib.rs"),
        "pub fn value() -> u8 { 42 }\n",
    )
    .unwrap();
    fs::write(root.join("build.rs"), r#"fn main() {
    std::fs::write(std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join("value.rs"), "pub const VALUE: u8 = 7;").unwrap();
    println!("cargo:rerun-if-changed=build.rs");
}"#).unwrap();
    fs::write(
        root.join("src/main.rs"),
        r#"include!(concat!(env!("OUT_DIR"), "/value.rs"));
fn main() { println!("{} {}", helper::value(), VALUE); }
"#,
    )
    .unwrap();
    let old = build(root, true, true);
    let old_incremental: Vec<_> = fs::read_dir(root.join("target/release/incremental"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_dir())
        .collect();
    assert!(!old_incremental.is_empty());
    let started = SystemTime::now();
    let current = build(root, false, true);
    let old_dependency = old
        .iter()
        .find(|m| m["reason"] == "compiler-artifact" && m["target"]["name"] == "helper")
        .unwrap()["filenames"][0]
        .as_str()
        .unwrap();
    assert!(Path::new(old_dependency).exists());
    write_messages(&root.join("old.json"), &old);
    assert!(
        dessplay::build_cache::prune(&root.join("old.json"), &root.join("target"), started)
            .is_err(),
        "a superseded report must not prune the newer build"
    );
    assert!(Path::new(old_dependency).exists());
    assert!(
        dessplay::build_cache::prune(&root.join("messages.json"), &root.join("target"), started)
            .unwrap()
            > 0
    );
    assert!(!Path::new(old_dependency).exists());
    for directory in old_incremental {
        assert!(
            !directory.exists(),
            "obsolete incremental directory: {}",
            directory.display()
        );
    }
    for message in current
        .iter()
        .filter(|m| m["reason"] == "compiler-artifact")
    {
        for filename in message["filenames"].as_array().unwrap() {
            assert!(Path::new(filename.as_str().unwrap()).exists());
        }
    }
    for tracking in [true, false, true] {
        let warm = build(root, false, tracking);
        assert!(
            warm.iter()
                .filter(|m| m["reason"] == "compiler-artifact")
                .all(|m| m["fresh"] == true),
            "cleanup and adding/removing the tracking wrapper must not cause even one compilation: {warm:#?}"
        );
    }
    assert!(!fs::read(root.join("wrapper.log")).unwrap().is_empty());
    let output = Command::new(root.join("target/release/cache-probe"))
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(output.stdout, b"42 7\n");
}

fn cached_graph() -> (
    tempfile::TempDir,
    std::path::PathBuf,
    std::path::PathBuf,
    SystemTime,
) {
    let temp = tempfile::tempdir().unwrap();
    let target = temp.path().join("target");
    for hash in [1u64, 2] {
        old_file(
            &target.join(format!("release/deps/libpkg-{hash:016x}.rlib")),
            b"cached",
        );
        let fingerprint = target.join(format!("release/.fingerprint/pkg-{hash:016x}/lib-pkg"));
        old_file(&fingerprint, b"0000000000000000");
        old_file(&fingerprint.with_extension("json"), b"{\"deps\":[]}");
    }
    let messages = temp.path().join("messages.json");
    write_messages(
        &messages,
        &[
            serde_json::json!({"reason":"compiler-artifact", "target":{"name":"pkg"}, "filenames":[target.join("release/deps/libpkg-0000000000000001.rlib")]}),
            serde_json::json!({"reason":"build-finished", "success":true}),
        ],
    );
    (
        temp,
        target,
        messages,
        SystemTime::now() + std::time::Duration::from_secs(2),
    )
}

#[test]
fn cargo_lock_contention_never_deletes_artifacts() {
    let (_temp, target, messages, started) = cached_graph();
    for lock in [".cargo-lock", ".cargo-build-lock", ".cargo-artifact-lock"] {
        let guard = fs::File::create(target.join("release").join(lock)).unwrap();
        guard.try_lock().unwrap();
        assert!(dessplay::build_cache::prune(&messages, &target, started).is_err());
        assert!(
            target
                .join("release/deps/libpkg-0000000000000002.rlib")
                .exists()
        );
    }
}

#[test]
fn incremental_shared_with_current_or_untracked_legacy_unit_is_preserved() {
    use std::os::unix::ffi::OsStrExt;
    for tracked in [false, true] {
        let (_temp, target, messages, started) = cached_graph();
        let incremental = target.join("release/incremental/pkg-shared");
        old_file(
            &incremental.join("session/cache"),
            b"valuable incremental state",
        );
        let mut record = incremental.as_os_str().as_bytes().to_vec();
        record.push(0);
        old_file(
            &target.join("release/deps/.dessplay-incremental-0000000000000002"),
            &record,
        );
        if tracked {
            old_file(
                &target.join("release/deps/.dessplay-incremental-0000000000000001"),
                &record,
            );
        }
        assert_eq!(
            dessplay::build_cache::prune(&messages, &target, started).unwrap(),
            1
        );
        assert!(incremental.join("session/cache").exists());
    }
}

#[test]
fn newer_units_and_symlinked_trees_are_not_swept() {
    for symlink in [false, true] {
        let (temp, target, messages, started) = cached_graph();
        if symlink {
            let external = temp.path().join("unrelated");
            old_file(&external.join("precious"), b"keep");
            fs::create_dir_all(target.join("release/build")).unwrap();
            std::os::unix::fs::symlink(
                &external,
                target.join("release/build/pkg-0000000000000002"),
            )
            .unwrap();
        } else {
            fs::File::options()
                .write(true)
                .open(target.join("release/deps/libpkg-0000000000000002.rlib"))
                .unwrap()
                .set_modified(started + std::time::Duration::from_secs(1))
                .unwrap();
        }
        assert_eq!(
            dessplay::build_cache::prune(&messages, &target, started).unwrap(),
            0
        );
        assert!(
            target
                .join("release/deps/libpkg-0000000000000002.rlib")
                .exists()
        );
        if symlink {
            assert!(temp.path().join("unrelated/precious").exists());
        }
    }
}
