//! Binding against the real standard library.
//!
//! Every other test binary loads its fixtures against the small test-only std in
//! `test-support/mini-sysroot`; the target runner leaves binaries named
//! `real_std` alone, so these load the toolchain's `rust-src`. Each case is one
//! whose result depends on std items, repeated from `name_fallback.rs` and
//! `resolution.rs`, which run it against the small std.

use std::fs;
use std::path::Path;

use unlace_adapter_rust::RustAdapter;
use unlace_ir::{Adapter, EdgeKind, Hardness, IrFragment, Node, SourceFile};

/// Parses and binds `paths` of the fixture under `tests/fixtures/<fixture>`,
/// plus the `extra` sources, which the files on disk need not match.
fn bind_fixture(
    fixture: &str,
    paths: &[&str],
    extra: &[(&str, &str)],
) -> Result<IrFragment, String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(fixture);
    let mut files = paths
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
    files.extend(extra.iter().map(|(path, contents)| SourceFile {
        path: (*path).into(),
        contents: (*contents).to_owned(),
    }));
    let adapter = RustAdapter::new(root.join("Cargo.toml"));
    let trees = adapter.parse(&files).map_err(|error| error.to_string())?;
    adapter.bind(trees).map_err(|error| error.to_string())
}

fn node<'a>(fragment: &'a IrFragment, name: &str) -> Result<&'a Node, String> {
    fragment
        .nodes
        .iter()
        .find(|node| node.name == name)
        .ok_or_else(|| format!("missing node {name}"))
}

#[test]
fn should_keep_marking_public_api_past_std_reexports() -> Result<(), String> {
    // the crate root re-exports `std::collections`, `std::fmt::Display` and
    // `std::vec::Vec` beside the workspace items the walk must still reach, and
    // must stop short of `a` in the private `stale_eof` module.
    let fragment = bind_fixture(
        "fallback/app",
        &["src/lib.rs", "src/inner.rs", "src/user.rs"],
        &[(
            "src/stale_eof.rs",
            "fn a() { spooky(); }\npub fn late() {}\n",
        )],
    )?;
    assert!(node(&fragment, "present")?.public_api);
    assert!(node(&fragment, "via_workspace_root")?.public_api);
    assert!(!node(&fragment, "a")?.public_api);
    Ok(())
}

#[test]
fn should_not_bind_a_std_macro_receiver_to_a_free_function() -> Result<(), String> {
    // `macro_caller` calls `local_value()` inside the real `assert_eq!`.
    let fragment = bind_fixture(
        "resolution/app",
        &["src/lib.rs", "src/render.rs", "src/unindexed.rs"],
        &[],
    )?;
    let source = node(&fragment, "macro_caller")?.id;
    let wrong_target = node(&fragment, "first")?.id;
    let preserved_target = node(&fragment, "local_value")?.id;

    assert!(
        !fragment
            .edges
            .iter()
            .any(|edge| edge.source == source && edge.target == wrong_target)
            && fragment.edges.iter().any(|edge| edge.source == source
                && edge.target == preserved_target
                && edge.kind == EdgeKind::Call
                && edge.hardness == Hardness::Hard),
        "macro receiver calls must not bind by name: {:?}",
        fragment.edges
    );
    Ok(())
}
