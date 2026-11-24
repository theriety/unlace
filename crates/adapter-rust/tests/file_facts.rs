//! Rust test-pair and entry facts, read from the module tree and cargo targets.

use std::fs;
use std::path::Path;

use unlace_adapter_rust::RustAdapter;
use unlace_ir::{Adapter, FileFact, SourceFile};

const FILES: [&str; 11] = [
    "build.rs",
    "src/lib.rs",
    "src/widget.rs",
    "src/widget/tests.rs",
    "src/support/check.rs",
    "src/main.rs",
    "src/bin/tool.rs",
    "src/bin/nested.rs",
    "tests/it.rs",
    "examples/demo.rs",
    "benches/speed.rs",
];

fn facts(paths: &[&str]) -> Result<Vec<FileFact>, String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/facts/app");
    let files = paths
        .iter()
        .map(|path| {
            fs::read_to_string(root.join(path))
                .map(|contents| SourceFile {
                    path: (*path).into(),
                    contents,
                })
                .map_err(|error| error.to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let adapter = RustAdapter::new(root.join("Cargo.toml"));
    let trees = adapter.parse(&files).map_err(|error| error.to_string())?;
    let fragment = adapter.bind(trees).map_err(|error| error.to_string())?;
    Ok(fragment.file_facts)
}

#[test]
fn should_pair_a_child_test_file_and_a_path_attribute_module_with_their_source()
-> Result<(), String> {
    let pairs: Vec<FileFact> = facts(&FILES)?
        .into_iter()
        .filter(|fact| fact.test_of.is_some())
        .collect();
    assert_eq!(
        pairs,
        vec![
            FileFact::test_pair("src/support/check.rs", "src/lib.rs"),
            FileFact::test_pair("src/widget/tests.rs", "src/widget.rs"),
        ]
    );
    Ok(())
}

#[test]
fn should_record_every_cargo_target_root_as_an_entry_and_nothing_else() -> Result<(), String> {
    let entries: Vec<String> = facts(&FILES)?
        .into_iter()
        .filter(|fact| fact.entry)
        .map(|fact| fact.file.to_string())
        .collect();
    assert_eq!(
        entries,
        [
            "benches/speed.rs",
            "build.rs",
            "examples/demo.rs",
            "src/bin/nested.rs",
            "src/bin/tool.rs",
            "src/lib.rs",
            "src/main.rs",
            "tests/it.rs",
        ]
    );
    Ok(())
}

#[test]
fn should_record_no_pair_when_the_owning_file_is_absent() -> Result<(), String> {
    let pairs = facts(&["src/widget/tests.rs"])?;
    assert_eq!(pairs, Vec::<FileFact>::new());
    Ok(())
}

#[test]
fn should_name_the_start_symbol_of_a_program_target_and_nothing_else() -> Result<(), String> {
    let starts: Vec<String> = facts(&FILES)?
        .into_iter()
        .filter(|fact| fact.start.is_some())
        .map(|fact| fact.file.to_string())
        .collect();
    // a binary, an example, a bench and a build script start a program; the
    // library's own `fn main`, the harness test target and the binary whose
    // only `main` sits inside a nested module (src/bin/nested.rs) add nothing.
    assert_eq!(
        starts,
        [
            "benches/speed.rs",
            "build.rs",
            "examples/demo.rs",
            "src/bin/tool.rs",
            "src/main.rs",
        ]
    );
    Ok(())
}

#[test]
fn should_name_the_start_of_a_target_whose_root_is_not_yet_on_disk() -> Result<(), String> {
    // the `tool` binary's root `src/tool.rs` exists only in the parsed source,
    // so its target path cannot be canonicalised and is compared as written.
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/facts/unwritten");
    let lib = fs::read_to_string(root.join("src/lib.rs")).map_err(|error| error.to_string())?;
    let adapter = RustAdapter::new(root.join("Cargo.toml"));
    let trees = adapter
        .parse(&[
            SourceFile {
                path: "src/lib.rs".into(),
                contents: lib,
            },
            SourceFile {
                path: "src/tool.rs".into(),
                contents: "fn main() {}\n".to_owned(),
            },
        ])
        .map_err(|error| error.to_string())?;

    let fragment = adapter.bind(trees).map_err(|error| error.to_string())?;

    assert_eq!(
        fragment.file_facts,
        vec![
            FileFact::entry_file("src/lib.rs"),
            FileFact::entry_with_start("src/tool.rs", "main"),
        ]
    );
    Ok(())
}
