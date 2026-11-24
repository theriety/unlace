//! Binding around names the semantic database cannot place: the name-based
//! fallback, re-exports of foreign or unplaced items, the public API walk, and
//! sources newer than the files on disk.
//!
//! The `fallback/app` fixture declares neither `orphan.rs` nor `loose.rs`, so
//! nothing in them resolves semantically; its `stale_*.rs` files on disk stop
//! short of the sources this test parses.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde_json::Value;
use unlace_adapter_rust::RustAdapter;
use unlace_ir::{Adapter, Edge, EdgeKind, Hardness, IrFragment, Node, ParseTree, SourceFile};

/// `src/stale_eof.rs` on disk ends where this source's `spooky` starts.
const STALE_EOF: &str = "fn a() { spooky(); }\npub fn late() {}\n";

/// `src/stale_far.rs` on disk ends before this source's `spooky` starts.
const STALE_FAR: &str = "fn c() {\n    spooky();\n}\n";

/// `src/stale_cut.rs` on disk ends right after this source's `spooky`.
const STALE_CUT: &str = "fn b() { spooky(); }\n";

/// `src/stale_pad.rs` on disk is only whitespace, as long as this source.
const STALE_PAD: &str = "\n\n\npub fn padded() {}\n";

/// `src/unwritten.rs` is not on disk at all, so the workspace has no file for it.
const UNWRITTEN: &str = "pub fn ghostly() { spooky(); }\n";

/// The confidence of an edge only the name-based fallback found.
const NAME_FALLBACK: f64 = 0.6;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fallback/app")
}

/// The adapter and the parse trees of the fixture, stale sources included.
fn parsed() -> Result<(RustAdapter, Vec<ParseTree>), String> {
    let root = root();
    let mut files = Vec::new();
    for path in [
        "src/lib.rs",
        "src/inner.rs",
        "src/orphan.rs",
        "src/loose.rs",
        "src/user.rs",
    ] {
        let contents = fs::read_to_string(root.join(path)).map_err(|error| error.to_string())?;
        files.push(SourceFile {
            path: path.into(),
            contents,
        });
    }
    for (path, contents) in [
        ("src/stale_eof.rs", STALE_EOF),
        ("src/stale_far.rs", STALE_FAR),
        ("src/stale_cut.rs", STALE_CUT),
        ("src/stale_pad.rs", STALE_PAD),
        ("src/unwritten.rs", UNWRITTEN),
    ] {
        files.push(SourceFile {
            path: path.into(),
            contents: contents.to_owned(),
        });
    }
    let adapter = RustAdapter::new(root.join("Cargo.toml"));
    let trees = adapter.parse(&files).map_err(|error| error.to_string())?;
    Ok((adapter, trees))
}

/// The bound fixture, shared by the tests that read it unchanged.
fn fragment() -> Result<&'static IrFragment, String> {
    static BOUND: OnceLock<Result<IrFragment, String>> = OnceLock::new();
    BOUND
        .get_or_init(|| {
            let (adapter, trees) = parsed()?;
            adapter.bind(trees).map_err(|error| error.to_string())
        })
        .as_ref()
        .map_err(Clone::clone)
}

fn node<'a>(fragment: &'a IrFragment, name: &str) -> Result<&'a Node, String> {
    fragment
        .nodes
        .iter()
        .find(|node| node.name == name)
        .ok_or_else(|| format!("missing node {name}"))
}

/// The edges from the node `source` to the node `target`.
fn edges<'a>(
    fragment: &'a IrFragment,
    source: &str,
    target: &str,
) -> Result<Vec<&'a Edge>, String> {
    let (source, target) = (node(fragment, source)?.id, node(fragment, target)?.id);
    Ok(fragment
        .edges
        .iter()
        .filter(|edge| edge.source == source && edge.target == target)
        .collect())
}

/// Whether `source` reaches `target` through a soft name-fallback edge of `kind`.
fn falls_back(
    fragment: &IrFragment,
    source: &str,
    target: &str,
    kind: EdgeKind,
) -> Result<bool, String> {
    Ok(edges(fragment, source, target)?.iter().any(|edge| {
        edge.kind == kind && edge.hardness == Hardness::Soft && edge.confidence == NAME_FALLBACK
    }))
}

#[test]
fn should_fall_back_on_a_qualifier_whose_root_is_a_workspace_module() -> Result<(), String> {
    // `inner::Ghost::make()`: `inner` resolves into the workspace, `Ghost` does not.
    let fragment = fragment()?;
    assert!(falls_back(
        fragment,
        "via_workspace_root",
        "Ghost",
        EdgeKind::TypeReference
    )?);
    Ok(())
}

#[test]
fn should_fall_back_on_a_qualifier_whose_unresolved_root_names_a_module_file() -> Result<(), String>
{
    // `orphan::Ghost::make()`: `orphan` is undeclared, but `src/orphan.rs` exists.
    let fragment = fragment()?;
    assert!(falls_back(
        fragment,
        "via_module_file",
        "Ghost",
        EdgeKind::TypeReference
    )?);
    Ok(())
}

#[test]
fn should_not_fall_back_on_a_foreign_unknown_global_or_macro_qualifier() -> Result<(), String> {
    let fragment = fragment()?;
    for source in [
        "via_external_root",
        "via_unknown_root",
        "via_global_path",
        "via_macro",
    ] {
        let found = edges(fragment, source, "Ghost")?;
        assert!(found.is_empty(), "{source}: {found:?}");
    }
    Ok(())
}

#[test]
fn should_fall_back_on_a_bare_call_spaced_from_its_arguments() -> Result<(), String> {
    let fragment = fragment()?;
    assert!(falls_back(
        fragment,
        "spaced_call",
        "spooky",
        EdgeKind::Call
    )?);
    Ok(())
}

#[test]
fn should_not_fall_back_on_a_reference_the_file_on_disk_does_not_reach() -> Result<(), String> {
    // in `stale_eof.rs` the reference starts at the end of the file on disk,
    // in `stale_far.rs` past it; in `stale_cut.rs` nothing follows it, so it
    // cannot be shown to be a call; `unwritten.rs` has no file on disk to
    // resolve or read it in.
    let fragment = fragment()?;
    for source in ["a", "c", "b", "ghostly"] {
        let found = edges(fragment, source, "spooky")?;
        assert!(found.is_empty(), "{source}: {found:?}");
    }
    Ok(())
}

#[test]
fn should_import_a_value_through_a_use_inside_a_body() -> Result<(), String> {
    let fragment = fragment()?;
    let found = edges(fragment, "imports", "present")?;
    assert!(
        found
            .iter()
            .any(|edge| edge.kind == EdgeKind::ValueImport && edge.hardness == Hardness::Hard),
        "{found:?}"
    );
    Ok(())
}

#[test]
fn should_emit_no_re_export_edge_for_a_foreign_unresolved_or_unplaced_target() -> Result<(), String>
{
    // `alias` names the module `inner`, whose first byte is no declaration.
    let fragment = fragment()?;
    for barrel in [
        "Byte",
        "Nothing",
        "alias",
        "collections",
        "Display",
        "StdVec",
    ] {
        let id = node(fragment, barrel)?.id;
        let found: Vec<_> = fragment
            .edges
            .iter()
            .filter(|edge| edge.source == id)
            .collect();
        assert!(found.is_empty(), "{barrel}: {found:?}");
    }
    Ok(())
}

#[test]
fn should_keep_marking_public_api_past_names_outside_the_workspace() -> Result<(), String> {
    // the crate root also re-exports std items, a builtin type, an unresolved
    // import, and `inner` a second time under `alias`.
    let fragment = fragment()?;
    assert!(node(fragment, "present")?.public_api);
    assert!(node(fragment, "via_workspace_root")?.public_api);
    assert!(!node(fragment, "a")?.public_api);
    Ok(())
}

#[test]
fn should_exempt_the_names_of_an_unloaded_file_without_declarations() -> Result<(), String> {
    // `loose.rs` is never loaded and only imports `present`.
    let fragment = fragment()?;
    assert!(node(fragment, "present")?.advice_exempt);
    assert!(!node(fragment, "spaced_call")?.advice_exempt);
    Ok(())
}

#[test]
fn should_state_no_rung_for_a_declaration_the_file_on_disk_does_not_hold() -> Result<(), String> {
    // `late` starts past the end of its file on disk, `padded` on its trailing
    // whitespace, and `ghostly` sits in a file the disk does not have.
    let fragment = fragment()?;
    for name in ["late", "padded", "ghostly"] {
        let id = node(fragment, name)?.id;
        let scope = fragment
            .visibility_scopes
            .iter()
            .find(|scope| scope.node == id)
            .ok_or_else(|| format!("no scope stated for {name}"))?;
        assert_eq!(scope.declared_rung, None, "{name}");
        assert!(scope.expressible.is_empty(), "{name}: {scope:?}");
    }
    Ok(())
}

#[test]
fn should_emit_no_edges_from_a_declaration_whose_span_is_empty() -> Result<(), String> {
    // a summary whose `imports` spans no byte owns no offset, so its `use`
    // reference has no source node to start from.
    let (adapter, mut trees) = parsed()?;
    let tree = trees
        .iter_mut()
        .find(|tree| tree.path == "src/user.rs")
        .ok_or("no tree for src/user.rs")?;
    let mut payload: Value =
        serde_json::from_str(&tree.payload).map_err(|error| error.to_string())?;
    let imports = payload
        .get_mut("declarations")
        .and_then(Value::as_array_mut)
        .and_then(|declarations| {
            declarations
                .iter_mut()
                .find(|declaration| declaration.get("name") == Some(&Value::from("imports")))
        })
        .ok_or("no imports declaration")?;
    let start = imports.get("byte_start").cloned().ok_or("no byte_start")?;
    imports
        .as_object_mut()
        .ok_or("declaration is no object")?
        .insert("byte_end".to_owned(), start);
    tree.payload = payload.to_string();

    let fragment = adapter.bind(trees).map_err(|error| error.to_string())?;

    let source = node(&fragment, "imports")?.id;
    let found: Vec<_> = fragment
        .edges
        .iter()
        .filter(|edge| edge.source == source)
        .collect();
    assert!(found.is_empty(), "{found:?}");
    Ok(())
}

#[test]
fn should_mark_only_the_pub_inherent_items_of_a_public_type_as_api() -> Result<(), String> {
    let fragment = fragment()?;
    assert!(node(fragment, "open")?.public_api);
    assert!(!node(fragment, "hidden")?.public_api);
    Ok(())
}
