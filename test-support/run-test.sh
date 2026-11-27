#!/bin/sh
# Cargo's target runner on Unix (see .cargo/config.toml): `cargo test`,
# `cargo nextest run` and `cargo run` start every binary through this script.
#
# Test binaries sit directly in a `deps` directory (`target/.../deps/`). For
# them it points rust-analyzer at the small standard library in `mini-sysroot/`
# through `RUST_SRC_PATH`, so a Rust-fixture load skips indexing the real std.
# `unlace` binaries a test spawns inherit the variable. Binaries named
# `real_std` keep the real std: they are the smoke tests that prove analysis
# still holds against it. `UNLACE_REAL_STD`, set to anything but empty or `0`,
# keeps the real std for every binary. Anything else, such as `cargo run`, runs
# untouched.
case ${UNLACE_REAL_STD:-0} in
0) ;;
*) exec "$@" ;;
esac
case ${1%/*} in
*/deps)
    case ${1##*/} in
    real_std-*) ;;
    *)
        RUST_SRC_PATH="$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)/mini-sysroot"
        export RUST_SRC_PATH
        ;;
    esac
    ;;
esac
exec "$@"
