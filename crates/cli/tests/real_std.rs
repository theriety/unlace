//! The `rust` e2e fixture against the real standard library.
//!
//! Every other test binary loads Rust against the small test-only std in
//! `test-support/mini-sysroot`; the target runner leaves binaries named
//! `real_std` alone, so this one, and the `unlace` it spawns, load the
//! toolchain's `rust-src`. It checks the library and the binary still agree,
//! and that the committed golden `parity` checks against the small std holds.

use std::path::{Path, PathBuf};
use std::process::Command;

use unlace_engine::{analyze, load_config, snapshot_from_root};

/// A config path that does not exist, so both sides run on the defaults.
const PURE_DEFAULTS: &str = "/nonexistent/unlace-real-std.toml";

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/e2e/fixtures/rust")
}

/// Runs `unlace analyze` over the fixture and returns its stdout.
fn run_analyze(extra: &[&str]) -> Result<Vec<u8>, String> {
    let output = Command::new(env!("CARGO_BIN_EXE_unlace"))
        .args(["analyze", "--root"])
        .arg(fixture())
        .args(["--config", PURE_DEFAULTS])
        .args(extra)
        .output()
        .map_err(|error| error.to_string())?;
    assert_eq!(output.status.code(), Some(0), "analyze exits 0");
    Ok(output.stdout)
}

#[test]
fn should_match_the_library_result_for_the_rust_fixture() -> Result<(), String> {
    let mut binary = run_analyze(&["--format", "json"])?;
    // the CLI appends one trailing newline the library serialization omits.
    assert_eq!(binary.pop(), Some(b'\n'));

    let config = load_config(PURE_DEFAULTS).unwrap_or_default();
    let result = snapshot_from_root(fixture(), &config)
        .and_then(|snapshot| analyze(&snapshot, &config))
        .map_err(|error| error.to_string())?;
    let library = serde_json::to_vec(&result).map_err(|error| error.to_string())?;
    assert!(
        binary == library,
        "cli json must be byte-identical to the library result"
    );
    Ok(())
}

#[test]
fn should_match_the_summary_golden_for_the_rust_fixture() -> Result<(), String> {
    let summary = run_analyze(&["--mode", "both", "--format", "summary"])?;
    let golden =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/e2e/fixtures/goldens/rust/summary.txt");
    let expected = std::fs::read(&golden).map_err(|error| error.to_string())?;
    assert_eq!(
        String::from_utf8_lossy(&summary),
        String::from_utf8_lossy(&expected)
    );
    Ok(())
}
