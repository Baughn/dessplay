//! Exercise the real launcher with isolated build/update tools, without network
//! access or a release build. Cargo runs the application in its own cwd.
#![cfg(unix)]

use std::{fs, os::unix::fs::PermissionsExt, path::Path, process::Command};

fn executable(path: &Path, script: &str) {
    fs::write(path, script).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

fn launcher_preserves_invocation(nix: bool) {
    for first_install in [false, true] {
        for track in [
            None,
            Some("master"),
            Some("stable"),
            Some("invalid"),
            Some(""),
        ] {
            for update in ["unchanged", "changed", "offline", "checkout-failed"] {
                let offline = update == "offline";
                let temp = tempfile::tempdir().unwrap();
                let home = temp.path().join("home with ' quotes");
                let repo = home.join(".cache/dessplay/repo");
                let caller = temp.path().join("caller with ' quotes $dollars `ticks`");
                let bin = temp.path().join("tools");
                fs::create_dir_all(&home).unwrap();
                fs::create_dir_all(&caller).unwrap();
                fs::create_dir_all(&bin).unwrap();
                std::os::unix::fs::symlink("/bin/sh", bin.join("sh")).unwrap();
                // Keep real Cargo/Nix/Git out of PATH, even on distributions that
                // install them in /usr/bin. Only these filesystem tools are real.
                for tool in ["mkdir", "cp", "ln", "sed", "cat", "cksum"] {
                    let path = std::env::split_paths(&std::env::var_os("PATH").unwrap())
                        .map(|dir| dir.join(tool))
                        .find(|path| path.is_file())
                        .unwrap();
                    std::os::unix::fs::symlink(fs::canonicalize(path).unwrap(), bin.join(tool))
                        .unwrap();
                }
                let source = temp.path().join("install.sh");
                executable(&source, include_str!("../../install.sh"));
                if !first_install {
                    fs::create_dir_all(repo.join(".git")).unwrap();
                    fs::write(repo.join("flake.nix"), "").unwrap();
                    fs::copy(&source, repo.join("install.sh")).unwrap();
                    fs::create_dir_all(home.join(".local/bin")).unwrap();
                    std::os::unix::fs::symlink(
                        repo.join("install.sh"),
                        home.join(".local/bin/dessplay"),
                    )
                    .unwrap();
                }
                executable(
                    &bin.join("git"),
                    r#"#!/bin/sh
set -eu
case "$1" in
    clone)
        mkdir -p "$TEST_REPO/.git"
        cp "$TEST_SOURCE" "$TEST_REPO/install.sh"
        : > "$TEST_REPO/flake.nix"
        ;;
    fetch)
        [ "$PWD" = "$TEST_REPO" ] || exit 91
        [ "$*" = "fetch --depth 1 origin +refs/heads/$TEST_TRACK:refs/remotes/origin/$TEST_TRACK" ] || exit 93
        printf '%s\n' "$PWD" >> "$TEST_UPDATE_LOG"
        [ "$TEST_UPDATE" != offline ] || exit 1
        ;;
    checkout)
        printf '%s' "$*" > "$TEST_CHECKOUT_LOG"
        [ "$TEST_UPDATE" != checkout-failed ] || exit 1
        if [ "$TEST_UPDATE" = changed ]; then
            cp "$TEST_UPDATED_SOURCE" "$TEST_REPO/install.sh"
        fi
        ;;
    *) exit 92 ;;
esac
"#,
                );
                executable(
                    &bin.join("cargo"),
                    "#!/bin/sh\n[ -z \"${DESSPLAY_LAUNCHER_REEXEC:-}\" ] || exit 94\nprintf '%s\\0' \"$PWD\" \"$@\"\nexit 23\n",
                );
                for tool in ["pkg-config", "mpv"] {
                    executable(&bin.join(tool), "#!/bin/sh\nexit 0\n");
                }
                if nix {
                    executable(
                        &bin.join("nix-shell"),
                        r#"#!/bin/sh
set -eu
[ "$1" = -E ] && [ "$3" = --run ] && [ "$#" = 4 ]
exec /bin/sh -c "$4"
"#,
                    );
                }
                let args = [
                    "layout",
                    "init",
                    "dir with ' quotes $(exit 99) `exit 98`",
                    "",
                ];
                let updated_source = temp.path().join("updated.sh");
                executable(
                    &updated_source,
                    &include_str!("../../install.sh").replace(
                        "set -eu",
                        "set -eu\nprintf restarted > \"$TEST_REEXEC_LOG\"",
                    ),
                );
                let reexec_log = temp.path().join("reexec.log");
                let checkout_log = temp.path().join("checkout.log");
                if let Some(track) = track {
                    fs::create_dir_all(home.join(".cache/dessplay")).unwrap();
                    fs::write(
                        home.join(".cache/dessplay/update-track"),
                        format!("{track}\n"),
                    )
                    .unwrap();
                }
                let update_log = temp.path().join("update.log");
                let output = Command::new(if first_install {
                    source.clone()
                } else {
                    home.join(".local/bin/dessplay")
                })
                .current_dir(&caller)
                .env_clear()
                .env("HOME", &home)
                .env("PATH", &bin)
                .env("TEST_REPO", &repo)
                .env("TEST_SOURCE", &source)
                .env("TEST_UPDATED_SOURCE", &updated_source)
                .env("TEST_REEXEC_LOG", &reexec_log)
                .env("TEST_CHECKOUT_LOG", &checkout_log)
                .env("TEST_UPDATE_LOG", &update_log)
                .env("TEST_UPDATE", update)
                .env("TEST_TRACK", track.unwrap_or("master"))
                .args(args)
                .output()
                .unwrap();
                if matches!(track, Some("invalid" | "")) {
                    assert_eq!(output.status.code(), Some(1));
                    assert!(
                        String::from_utf8_lossy(&output.stderr).contains("invalid update track")
                    );
                    assert!(!update_log.exists());
                    assert!(output.stdout.is_empty());
                    continue;
                }
                assert_eq!(
                    output.status.code(),
                    Some(23),
                    "nix={nix}, first_install={first_install}, offline={offline}: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
                assert_eq!(
                    fs::read_to_string(update_log).unwrap(),
                    format!("{}\n", repo.display())
                );
                assert_eq!(
                    reexec_log.exists(),
                    update == "changed",
                    "updated launcher must execute before building"
                );
                if !offline {
                    assert_eq!(
                        fs::read_to_string(&checkout_log).unwrap(),
                        format!(
                            "checkout --detach refs/remotes/origin/{}",
                            track.unwrap_or("master")
                        )
                    );
                }
                let stdout = String::from_utf8(output.stdout).unwrap();
                let fields: Vec<_> = stdout.split_terminator('\0').collect();
                // macOS's /tmp can be a symlink; compare filesystem identities.
                assert_eq!(
                    fs::canonicalize(fields[0]).unwrap(),
                    fs::canonicalize(&caller).unwrap(),
                    "application cwd: nix={nix}, first_install={first_install}, offline={offline}"
                );
                assert_eq!(
                    &fields[1..],
                    [
                        "run",
                        "--release",
                        "--manifest-path",
                        repo.join("Cargo.toml").to_str().unwrap(),
                        "-p",
                        "dessplay",
                        "--",
                        "--update-track-file",
                        home.join(".cache/dessplay/update-track").to_str().unwrap(),
                        args[0],
                        args[1],
                        args[2],
                        args[3],
                    ]
                );
            }
        }
    }
}

#[test]
fn system_cargo_preserves_caller_directory_and_arguments() {
    launcher_preserves_invocation(false);
}

#[test]
fn nix_shell_preserves_caller_directory_and_arguments() {
    launcher_preserves_invocation(true);
}
