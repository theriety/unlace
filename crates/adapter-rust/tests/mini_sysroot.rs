//! The test-only standard library in `test-support/mini-sysroot` keeps step
//! with rust-analyzer.
//!
//! Its `core` embeds rust-analyzer's `minicore` between two marker lines. This
//! test rebuilds that part from the `ra_ap_test_utils` in `Cargo.lock` and fails
//! when the committed copy differs, so an `ra_ap_*` upgrade cannot leave a stale
//! library behind. With `UNLACE_BLESS_MINICORE=1` set it rewrites a stale part
//! instead. The variable is separate from the goldens' `UNLACE_BLESS`, so blessing
//! goldens never regenerates the vendored `core` with them.

use std::fs;
use std::path::{Path, PathBuf};

use ra_ap_test_utils::MiniCore;

/// The line before the embedded `minicore`.
const BEGIN: &str = "// BEGIN minicore\n";

/// The line after the embedded `minicore`.
const END: &str = "// END minicore\n";

fn core_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-support/mini-sysroot/core/src/lib.rs")
}

/// `minicore` with every flag on, except the `fmt_before_*` flags, which swap
/// in the `fmt` internals of older toolchains.
fn minicore() -> String {
    let flags =
        MiniCore::available_flags(MiniCore::RAW_SOURCE).filter(|flag| !flag.contains("_before_"));
    MiniCore::from_flags(flags).source_code(MiniCore::RAW_SOURCE)
}

#[test]
fn should_embed_the_minicore_of_the_locked_rust_analyzer() -> Result<(), String> {
    let path = core_path();
    let committed =
        fs::read_to_string(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    let (head, rest) = committed
        .split_once(BEGIN)
        .ok_or_else(|| format!("{} lacks the line {BEGIN:?}", path.display()))?;
    let (embedded, tail) = rest
        .split_once(END)
        .ok_or_else(|| format!("{} lacks the line {END:?}", path.display()))?;
    let current = minicore();

    if embedded != current && std::env::var_os("UNLACE_BLESS_MINICORE").is_some() {
        let blessed = format!("{head}{BEGIN}{current}{END}{tail}");
        return fs::write(&path, blessed).map_err(|error| format!("{}: {error}", path.display()));
    }
    let stale_line = embedded
        .lines()
        .zip(current.lines())
        .position(|(old, new)| old != new)
        .unwrap_or_else(|| embedded.lines().count().min(current.lines().count()));
    assert!(
        embedded == current,
        "the minicore in {} is stale from its line {} on; regenerate it with \
         `UNLACE_BLESS_MINICORE=1 cargo test -p unlace-adapter-rust --test mini_sysroot`",
        path.display(),
        stale_line + 1
    );
    Ok(())
}
