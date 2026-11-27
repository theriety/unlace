//! Analysis against the real standard library.
//!
//! Every other test binary loads Rust against the small test-only std in
//! `test-support/mini-sysroot`; the target runner leaves binaries named
//! `real_std` alone, so this one loads the toolchain's `rust-src`.

use std::fs;
use std::path::PathBuf;

use unlace_engine::{AnalyzeConfig, analyze, snapshot_from_root};

/// Writes a throwaway crate holding `files` and returns its root.
fn write_crate(files: &[(&str, &str)]) -> Result<PathBuf, String> {
    let root = std::env::temp_dir().join(format!("unlace-real-std-{}", std::process::id()));
    let manifest = "[package]\nname = \"real\"\nversion = \"0.1.0\"\nedition = \"2021\"\n";
    for (path, contents) in files.iter().chain(&[("Cargo.toml", manifest)]) {
        let target = root.join(path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        fs::write(&target, contents).map_err(|error| error.to_string())?;
    }
    Ok(root)
}

#[test]
fn should_flag_a_pub_crate_item_used_only_inside_a_std_macro_of_its_module() -> Result<(), String> {
    // `helper`'s one use sits inside the real `assert_eq!`, in its own module.
    let root = write_crate(&[
        ("src/lib.rs", "mod a;\nfn go() {\n    a::run();\n}\n"),
        (
            "src/a.rs",
            "pub(crate) fn run() {\n    assert_eq!(helper(), 1);\n}\npub(crate) fn helper() -> u32 {\n    1\n}\n",
        ),
    ])?;
    let config = AnalyzeConfig::default();
    let result = snapshot_from_root(&root, &config)
        .and_then(|snapshot| analyze(&snapshot, &config))
        .map_err(|error| error.to_string());
    let _ = fs::remove_dir_all(&root);
    let json = serde_json::to_string_pretty(&result?).map_err(|error| error.to_string())?;
    let advice: Vec<&str> = json
        .lines()
        .filter(|line| line.contains("exported at"))
        .collect();

    let helper = advice
        .iter()
        .find(|line| line.contains("`helper`"))
        .ok_or_else(|| format!("helper must be flagged: {advice:?}"))?;
    assert!(
        helper.contains("narrowest spelling: private (no `pub`)"),
        "{helper}"
    );
    assert!(
        !advice.iter().any(|line| line.contains("`run`")),
        "{advice:?}"
    );
    Ok(())
}
