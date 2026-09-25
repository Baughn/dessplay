#!/bin/sh
# Cargo target runner (macOS; wired up in .cargo/config.toml). Cargo and
# nextest launch every test binary through it.
#
# Binaries under target/*/deps/ run from an APFS clone in a temp
# directory instead of in place. On macOS every FSEvents stream
# registration (notify's backend restarts its stream on each
# watch/unwatch) costs ~0.7s when the running executable's real path is
# in a huge directory, and deps/ keeps every old codegen unit's .o file
# for debug info: ~480k entries after a long dev session, until `cargo
# clean`. That timed out the layout live-reload tests.
#
# It must be a real file elsewhere: a symlink or hardlink still resolves
# to the deps/ path and stays slow (measured 2026-09-26). A clone
# (`/bin/cp -c`, clonefile(2)) is instant and shares blocks with the
# original until the linker replaces it. Debug info still works: the
# binary's debug map names the .o files by absolute path.
#
# Anything else (`cargo run` of target/*/dessplay) runs untouched.
set -eu
bin=$1
shift
case $bin in
*/deps/*) ;;
*) exec "$bin" "$@" ;;
esac
abs=$(cd "$(dirname "$bin")" && pwd -P)/$(basename "$bin")
# Mirror the absolute path so checkouts and profiles never share a clone.
clone=${TMPDIR:-/tmp}/dessplay-test-bins$abs
if [ ! -e "$clone" ] || [ "$abs" -nt "$clone" ]; then
    mkdir -p "$(dirname "$clone")"
    # Parallel test processes race here: clone to a private name, then
    # rename into place atomically. Nix's coreutils cp has no -c, hence
    # the absolute path.
    tmp=$clone.$$
    if /bin/cp -c "$abs" "$tmp" 2>/dev/null; then
        mv -f "$tmp" "$clone"
    else
        rm -f "$tmp"
        exec "$abs" "$@"
    fi
fi
exec "$clone" "$@"
