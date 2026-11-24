//! Binding inputs at the edges of what the semantic database can place: a file
//! module under a macro-defined inline module, a reference whose position on
//! disk falls in a doc comment, and a parse summary with a restricted
//! visibility that names no path.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;
use smol_str::SmolStr;
use unlace_adapter_rust::RustAdapter;
use unlace_ir::{Adapter, IrFragment, ParseTree, SourceFile, VisibilityScope};

/// The source parsed for `src/docs.rs`: on disk, `spooky` sits inside the
/// leading doc comment, which names no item.
const DOCS: &str = "pub fn d() { spooky(); }\n";

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/edge_inputs/app")
}

/// The adapter and the parse trees of the fixture, `docs.rs` newer than disk.
fn parsed() -> Result<(RustAdapter, Vec<ParseTree>), String> {
    let root = root();
    let mut files = Vec::new();
    for path in ["src/lib.rs", "src/outer/inner.rs"] {
        let contents = fs::read_to_string(root.join(path)).map_err(|error| error.to_string())?;
        files.push(SourceFile {
            path: path.into(),
            contents,
        });
    }
    files.push(SourceFile {
        path: "src/docs.rs".into(),
        contents: DOCS.to_owned(),
    });
    let adapter = RustAdapter::new(root.join("Cargo.toml"));
    let trees = adapter.parse(&files).map_err(|error| error.to_string())?;
    Ok((adapter, trees))
}

fn bind(adapter: &RustAdapter, trees: Vec<ParseTree>) -> Result<IrFragment, String> {
    adapter.bind(trees).map_err(|error| error.to_string())
}

/// The scope statement of the declaration called `name`.
fn scope_named<'a>(fragment: &'a IrFragment, name: &str) -> Result<&'a VisibilityScope, String> {
    let node = fragment
        .nodes
        .iter()
        .find(|node| node.name.as_str() == name)
        .ok_or_else(|| format!("no node named {name}"))?;
    fragment
        .visibility_scopes
        .iter()
        .find(|scope| scope.node == node.id)
        .ok_or_else(|| format!("no scope stated for {name}"))
}

/// The files of each rung of `scope`.
fn rung_files(scope: &VisibilityScope) -> Vec<Vec<&str>> {
    scope
        .expressible
        .iter()
        .map(|rung| rung.files.iter().map(SmolStr::as_str).collect())
        .collect()
}

#[test]
fn should_state_only_the_own_rung_below_a_macro_defined_inline_module() -> Result<(), String> {
    // `outer` has no file of its own and the crate root's syntax does not show
    // it, so neither is a rung: only `inner` itself, spelled private.
    let (adapter, trees) = parsed()?;
    let fragment = bind(&adapter, trees)?;

    let scope = scope_named(&fragment, "deep")?;

    assert_eq!(rung_files(scope), [["src/outer/inner.rs"]]);
    assert_eq!(
        scope
            .expressible
            .iter()
            .map(|rung| rung.spelling.as_deref())
            .collect::<Vec<_>>(),
        [Some("private (no `pub`)")]
    );
    Ok(())
}

#[test]
fn should_emit_no_edge_for_a_reference_that_sits_in_a_doc_comment_on_disk() -> Result<(), String> {
    // the database finds no definition at a doc comment without a link, and the
    // comment is not the name, so the unique `spooky` is not bound by name.
    let (adapter, trees) = parsed()?;
    let fragment = bind(&adapter, trees)?;

    let source = fragment
        .nodes
        .iter()
        .find(|node| node.name == "d")
        .ok_or("no node named d")?
        .id;
    let found: Vec<_> = fragment
        .edges
        .iter()
        .filter(|edge| edge.source == source)
        .collect();
    assert!(found.is_empty(), "{found:?}");
    Ok(())
}

#[test]
fn should_leave_a_restricted_declaration_without_a_path_unresolved() -> Result<(), String> {
    // a summary stating `pub(in ..)` with no path names no target module.
    let (adapter, mut trees) = parsed()?;
    let tree = trees
        .iter_mut()
        .find(|tree| tree.path == "src/outer/inner.rs")
        .ok_or("no tree for src/outer/inner.rs")?;
    let mut payload: Value =
        serde_json::from_str(&tree.payload).map_err(|error| error.to_string())?;
    payload
        .get_mut("declarations")
        .and_then(Value::as_array_mut)
        .and_then(|declarations| {
            declarations
                .iter_mut()
                .find(|declaration| declaration.get("name") == Some(&Value::from("lifted")))
        })
        .and_then(Value::as_object_mut)
        .ok_or("no lifted declaration")?
        .insert("visibility_path".to_owned(), Value::Null);
    tree.payload = payload.to_string();

    let fragment = bind(&adapter, trees)?;

    let scope = scope_named(&fragment, "lifted")?;
    assert!(scope.expressible.is_empty(), "{scope:?}");
    assert_eq!(scope.declared_rung, None);
    Ok(())
}
