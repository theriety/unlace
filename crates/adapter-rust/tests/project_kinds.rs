//! Projects that are not cargo workspaces, and paths that are no project.

use std::fs;
use std::path::Path;

use unlace_adapter_rust::RustAdapter;
use unlace_ir::{Adapter, AdapterError, FileFact, SourceFile};

#[test]
fn should_bind_a_rust_project_json_with_no_targets_and_every_root_a_library() -> Result<(), String>
{
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/project_json");
    let contents =
        fs::read_to_string(root.join("src/lib.rs")).map_err(|error| error.to_string())?;
    let adapter = RustAdapter::new(root.join("rust-project.json"));
    let trees = adapter
        .parse(&[SourceFile {
            path: "src/lib.rs".into(),
            contents,
        }])
        .map_err(|error| error.to_string())?;

    let fragment = adapter.bind(trees).map_err(|error| error.to_string())?;

    // with no cargo targets there are no entry files and no test-support crates,
    // while the unknown crate kind counts as a library whose `pub` item is API.
    // The module tree still makes `src/lib.rs` the entry of its directory.
    assert_eq!(
        fragment.file_facts,
        vec![FileFact::entry_of_folder("src/lib.rs", "src")]
    );
    assert!(fragment.test_support_roots.is_empty());
    let exported = fragment.nodes.iter().find(|node| node.name == "exported");
    assert_eq!(exported.map(|node| node.public_api), Some(true));
    Ok(())
}

#[test]
fn should_fail_to_bind_a_directory_that_holds_no_project() -> Result<(), String> {
    let empty = std::env::temp_dir().join(format!("unlace-no-project-{}", std::process::id()));
    fs::create_dir_all(&empty).map_err(|error| error.to_string())?;
    let adapter = RustAdapter::new(empty.join("src"));

    let result = adapter.bind(Vec::new());

    let _ = fs::remove_dir_all(&empty);
    assert!(
        matches!(&result, Err(AdapterError::Bind { reason, .. })
            if reason.starts_with("failed to load cargo workspace: ")),
        "{result:?}"
    );
    Ok(())
}
