# Test-only standard library

A small `core`, `alloc` and `std` that rust-analyzer loads in place of the real standard library while the workspace's tests run. With the real one a Rust-fixture load costs about 1.7 s, most of it indexing std on the first query; with this one it costs about 0.2 s. See "Tests use a small standard library" in [`CONTRIBUTE.md`](../../CONTRIBUTE.md). Each crate is `<name>/src/lib.rs` with no `Cargo.toml`, the layout rust-analyzer expects when `RUST_SRC_PATH` names a directory.

[`../run-test.sh`](../run-test.sh), the target runner in [`.cargo/config.toml`](../../.cargo/config.toml), sets `RUST_SRC_PATH` to this directory for every test binary except those named `real_std`, and for none when `UNLACE_REAL_STD=1` is set. `cargo run` and installed binaries never see it.

- `core/src/lib.rs` embeds rust-analyzer's `minicore` (from the `ra_ap_test_utils` crate, MIT OR Apache-2.0, see [`LICENSE-MIT`](LICENSE-MIT)) with every flag on except `fmt_before_*`, followed by the builtin attributes and macros the real `core` has and `minicore` lacks.
- `alloc/src/lib.rs` and `std/src/lib.rs` are unlace's own: just the items the fixtures name, with bodies left out. Macros must expand their arguments like the real ones, or references inside `assert_eq!` and friends stop resolving.

After an `ra_ap_*` upgrade, the `mini_sysroot` test in `unlace-adapter-rust` fails until you regenerate the embedded `minicore`:

```sh
UNLACE_BLESS_MINICORE=1 cargo test -p unlace-adapter-rust --test mini_sysroot
```
