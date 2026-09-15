#!/bin/sh
# Record incremental-cache ownership without changing any rustc arguments.
# RUSTC_WRAPPER does not change Cargo's artifact hashes (unlike
# RUSTC_WORKSPACE_WRAPPER). Preserve an existing wrapper such as sccache.
set -eu

crate_name= out_dir= incremental= extra= previous=
for argument do
    case "$previous" in
        --crate-name) crate_name=$argument ;;
        --out-dir) out_dir=$argument ;;
        -C)
            case "$argument" in
                incremental=*) incremental=${argument#incremental=} ;;
                extra-filename=*) extra=${argument#extra-filename=-} ;;
            esac ;;
    esac
    case "$argument" in
        -Cincremental=*) incremental=${argument#-Cincremental=} ;;
        -Cextra-filename=*) extra=${argument#-Cextra-filename=-} ;;
    esac
    previous=$argument
done

stamp= record=
cleanup() {
    [ -z "$stamp" ] || rm -f "$stamp"
    [ -z "$record" ] || rm -f "$record"
}
trap cleanup 0
# Tracking is best effort. Failure must never turn a successful compilation
# into a failed build or discard an older ownership record.
case "$extra" in
    ''|*[!0-9a-f]*) ;;
    *)
        if [ "${#extra}" = 16 ] && [ -n "$crate_name" ] && [ -d "$incremental" ] && [ -d "$out_dir" ]; then
            stamp=$(mktemp "$incremental/.dessplay-start.XXXXXX") || stamp=
        fi ;;
esac
if [ -n "${DESSPLAY_ORIGINAL_RUSTC_WRAPPER:-}" ]; then
    "$DESSPLAY_ORIGINAL_RUSTC_WRAPPER" "$@"
else
    "$@"
fi
if [ -n "$stamp" ]; then
    record=$(mktemp "$out_dir/.dessplay-incremental-$extra.XXXXXX") || exit 0
    # rustc creates/renames a session inside its crate-specific directory on
    # every successful incremental compile. Concurrent compilations may add
    # extra directories; the pruner retains the union for all live units.
    for directory in "$incremental/$crate_name-"*; do
        [ -d "$directory" ] || continue
        find "$directory" -prune -newer "$stamp" -print0 >> "$record" || exit 0
    done
    if [ -s "$record" ]; then
        mv -f "$record" "$out_dir/.dessplay-incremental-$extra" || exit 0
        record=
    fi
fi
