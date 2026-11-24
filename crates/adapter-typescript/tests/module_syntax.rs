//! How module-level syntax turns into nodes, edges, and containers: default
//! exports, string export names, declarations the adapter does not model, and
//! the edges calls and loads produce.

use smol_str::SmolStr;
use unlace_adapter_typescript::TypeScriptAdapter;
use unlace_ir::{Adapter, EdgeKind, Hardness, IrFragment, NodeId, NodeKind, SourceFile};

fn bind_files(files: &[(&str, &str)]) -> Result<IrFragment, String> {
    let adapter = TypeScriptAdapter::new(".");
    let sources: Vec<SourceFile> = files
        .iter()
        .map(|(path, contents)| SourceFile {
            path: SmolStr::new(*path),
            contents: (*contents).to_owned(),
        })
        .collect();
    let trees = adapter
        .parse(&sources)
        .map_err(|error| format!("sources did not parse: {error}"))?;

    adapter
        .bind(trees)
        .map_err(|error| format!("sources did not bind: {error}"))
}

fn bind_inline(contents: &str) -> Result<IrFragment, String> {
    bind_files(&[("src/module.ts", contents)])
}

fn node_id(fragment: &IrFragment, name: &str) -> Result<NodeId, String> {
    fragment
        .nodes
        .iter()
        .find(|node| node.name == name)
        .map_or_else(|| Err(format!("expected node {name}")), |node| Ok(node.id))
}

/// The sorted `(target, kind, hardness)` of every edge leaving `source`.
fn edges_from(
    fragment: &IrFragment,
    source: &str,
) -> Result<Vec<(SmolStr, EdgeKind, Hardness)>, String> {
    let source = node_id(fragment, source)?;
    let mut edges = fragment
        .edges
        .iter()
        .filter(|edge| edge.source == source)
        .map(|edge| {
            fragment
                .nodes
                .iter()
                .find(|node| node.id == edge.target)
                .map_or_else(
                    || Err(format!("expected edge target {:?}", edge.target)),
                    |node| Ok((node.name.clone(), edge.kind, edge.hardness)),
                )
        })
        .collect::<Result<Vec<_>, String>>()?;
    edges.sort();
    Ok(edges)
}

/// The `(name, kind, public_api)` of every node, in emission order.
fn nodes_of(fragment: &IrFragment) -> Vec<(&str, NodeKind, bool)> {
    fragment
        .nodes
        .iter()
        .map(|node| (node.name.as_str(), node.kind, node.public_api))
        .collect()
}

#[test]
fn should_name_an_anonymous_default_class_default_and_keep_its_dependencies() -> Result<(), String>
{
    let fragment = bind_inline(
        "class Base {}\nfunction helper(): void {}\nexport default class extends Base { run(): void { helper(); } }\n",
    )?;

    assert_eq!(
        nodes_of(&fragment),
        [
            ("Base", NodeKind::Symbol, false),
            ("helper", NodeKind::Symbol, false),
            ("default", NodeKind::Symbol, true),
        ]
    );
    assert_eq!(
        edges_from(&fragment, "default")?,
        vec![
            (SmolStr::new("Base"), EdgeKind::Inheritance, Hardness::Hard),
            (SmolStr::new("helper"), EdgeKind::Call, Hardness::Hard),
        ]
    );
    Ok(())
}

#[test]
fn should_keep_the_name_of_a_named_default_class() -> Result<(), String> {
    let fragment = bind_inline("export default class Widget {}\n")?;

    assert_eq!(nodes_of(&fragment), [("Widget", NodeKind::Symbol, true)]);
    Ok(())
}

#[test]
fn should_name_an_anonymous_default_function_default_and_keep_its_calls() -> Result<(), String> {
    let fragment = bind_inline(
        "function helper(): void {}\nexport default function (): void { helper(); }\n",
    )?;

    assert_eq!(
        edges_from(&fragment, "default")?,
        vec![(SmolStr::new("helper"), EdgeKind::Call, Hardness::Hard)]
    );
    Ok(())
}

#[test]
fn should_bind_a_default_interface_as_an_exported_type() -> Result<(), String> {
    let fragment =
        bind_inline("interface Base {}\nexport default interface Shape extends Base {}\n")?;

    assert_eq!(
        nodes_of(&fragment),
        [
            ("Base", NodeKind::Type, false),
            ("Shape", NodeKind::Type, true),
        ]
    );
    assert_eq!(
        edges_from(&fragment, "Shape")?,
        vec![(SmolStr::new("Base"), EdgeKind::Inheritance, Hardness::Hard)]
    );
    Ok(())
}

#[test]
fn should_re_export_a_binding_named_by_a_string_literal() -> Result<(), String> {
    let fragment = bind_files(&[
        (
            "src/index.ts",
            "export { \"widget\" as tool } from './widget';\n",
        ),
        ("src/widget.ts", "export function widget(): void {}\n"),
    ])?;

    assert_eq!(
        edges_from(&fragment, "tool")?,
        vec![(SmolStr::new("widget"), EdgeKind::ReExport, Hardness::Soft)]
    );
    Ok(())
}

#[test]
fn should_declare_nothing_for_a_global_namespace_export_or_empty_statements() -> Result<(), String>
{
    let fragment =
        bind_inline("export as namespace Library;\n;\nexport function api(): void {}\n")?;

    assert_eq!(nodes_of(&fragment), [("api", NodeKind::Symbol, true)]);
    Ok(())
}

#[test]
fn should_declare_no_node_for_a_destructured_binding() -> Result<(), String> {
    let fragment =
        bind_inline("const source = { item: 1 };\nconst { item } = source, after = item;\n")?;

    assert_eq!(
        nodes_of(&fragment),
        [
            ("source", NodeKind::Symbol, false),
            ("after", NodeKind::Symbol, false),
        ]
    );
    Ok(())
}

#[test]
fn should_not_treat_a_qualified_supertype_as_inheritance() -> Result<(), String> {
    let fragment = bind_inline(
        "interface Base {}\nconst shapes = { Base: 1 };\ninterface Child extends shapes.Base {}\n",
    )?;

    assert_eq!(edges_from(&fragment, "Child")?, []);
    Ok(())
}

#[test]
fn should_emit_one_call_edge_for_a_repeated_call() -> Result<(), String> {
    let fragment = bind_inline(
        "function helper(): void {}\nfunction run(): void { helper(); undeclared(); helper(); }\n",
    )?;

    assert_eq!(
        edges_from(&fragment, "run")?,
        vec![(SmolStr::new("helper"), EdgeKind::Call, Hardness::Hard)]
    );
    Ok(())
}

#[test]
fn should_emit_no_edge_for_a_recursive_call() -> Result<(), String> {
    let fragment = bind_inline("function walk(depth: number): void { walk(depth - 1); }\n")?;

    assert_eq!(edges_from(&fragment, "walk")?, []);
    Ok(())
}

#[test]
fn should_skip_an_unresolved_dynamic_import_and_still_link_the_next_one() -> Result<(), String> {
    let fragment = bind_files(&[
        (
            "src/loader.ts",
            "export async function load(): Promise<void> { await import('./missing'); await import('./present'); }\n",
        ),
        ("src/present.ts", "export function present(): void {}\n"),
    ])?;
    let load = node_id(&fragment, "load")?;
    let present = node_id(&fragment, "present")?;
    let dynamic: Vec<(NodeId, u64)> = fragment
        .edges
        .iter()
        .filter(|edge| edge.source == load)
        .map(|edge| (edge.target, edge.confidence.to_bits()))
        .collect();

    assert_eq!(dynamic, [(present, 0.5_f64.to_bits())]);
    Ok(())
}

#[test]
fn should_exempt_a_namespace_import_after_an_unresolved_one() -> Result<(), String> {
    let fragment = bind_files(&[
        (
            "src/app.ts",
            "import * as external from 'external';\nimport * as util from './util';\nexport function run(): void { util.help(); external.go(); }\n",
        ),
        ("src/util.ts", "export function help(): void {}\n"),
    ])?;
    let exempt: Vec<(&str, bool)> = fragment
        .nodes
        .iter()
        .map(|node| (node.name.as_str(), node.advice_exempt))
        .collect();

    assert_eq!(exempt, [("run", false), ("help", true)]);
    Ok(())
}

#[test]
fn should_hang_a_root_level_file_under_the_workspace_bucket() -> Result<(), String> {
    let fragment = bind_files(&[("main.ts", "export function main(): void {}\n")])?;
    let mut chain = Vec::new();
    let mut current = fragment
        .nodes
        .first()
        .map(|node| node.container)
        .ok_or_else(|| "expected one node".to_owned())?;
    while let Some(container) = fragment.containers.iter().find(|c| c.id == current) {
        chain.push(container.name.to_string());
        let Some(parent) = container.parent else {
            break;
        };
        current = parent;
    }

    assert_eq!(
        chain,
        ["main.ts", "workspace", "workspace", "workspace", "root"]
    );
    Ok(())
}
