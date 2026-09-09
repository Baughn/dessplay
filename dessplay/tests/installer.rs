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
        for offline in [false, true] {
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
            for tool in ["mkdir", "cp", "ln", "sed"] {
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
    pull)
        [ "$PWD" = "$TEST_REPO" ] || exit 91
        printf '%s' "$PWD" > "$TEST_UPDATE_LOG"
        exit "$TEST_PULL_STATUS"
        ;;
    *) exit 92 ;;
esac
"#,
            );
            executable(
                &bin.join("cargo"),
                "#!/bin/sh\nprintf '%s\\0' \"$PWD\" \"$@\"\nexit 23\n",
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
            .env("TEST_UPDATE_LOG", &update_log)
            .env("TEST_PULL_STATUS", if offline { "1" } else { "0" })
            .args(args)
            .output()
            .unwrap();
            assert_eq!(
                output.status.code(),
                Some(23),
                "nix={nix}, first_install={first_install}, offline={offline}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(
                fs::read_to_string(update_log).unwrap(),
                repo.to_str().unwrap()
            );
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
                    args[0],
                    args[1],
                    args[2],
                    args[3],
                ]
            );
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
