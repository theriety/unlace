//! References a value declaration's signature and body collect from type syntax:
//! every type a function or class names becomes an edge, whatever construct
//! names it.

use smol_str::SmolStr;
use unlace_adapter_typescript::TypeScriptAdapter;
use unlace_ir::{Adapter, EdgeKind, Hardness, IrFragment, NodeId, SourceFile};

fn bind_inline(contents: &str) -> Result<IrFragment, String> {
    let adapter = TypeScriptAdapter::new(".");
    let files = [SourceFile {
        path: SmolStr::new("src/body.ts"),
        contents: contents.to_string(),
    }];
    let trees = adapter
        .parse(&files)
        .map_err(|error| format!("inline source did not parse: {error}"))?;

    adapter
        .bind(trees)
        .map_err(|error| format!("inline source did not bind: {error}"))
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

fn type_references(names: &[&str]) -> Vec<(SmolStr, EdgeKind, Hardness)> {
    names
        .iter()
        .map(|name| (SmolStr::new(name), EdgeKind::TypeReference, Hardness::Soft))
        .collect()
}

#[test]
fn should_reference_every_member_signature_type_of_a_parameter_object_type() -> Result<(), String> {
    let fragment = bind_inline(
        "interface Local {} interface Other {} interface Ret {} interface CallRet {} interface Made {}\n\
         function accept(input: { get value(): Local; set value(next: Other); method(): Ret; (): CallRet; new (): Made }): void {}\n",
    )?;

    assert_eq!(
        edges_from(&fragment, "accept")?,
        type_references(&["CallRet", "Local", "Made", "Other", "Ret"])
    );
    Ok(())
}

#[test]
fn should_reference_every_type_inside_composite_parameter_and_return_types() -> Result<(), String> {
    let fragment = bind_inline(
        "interface In {} interface Out {} interface Holder {} interface Product {} interface Label {} type Keys = 'a'; interface Yes {} interface No {} interface Probe {} interface Bound {}\n\
         function shape(map: (input: In) => Out, make: new () => Product, pair: [label: Label], table: { [Key in Keys]: Yes }, pick: Probe extends Bound ? Yes : No, { item }: Holder): void {}\n",
    )?;

    assert_eq!(
        edges_from(&fragment, "shape")?,
        type_references(&[
            "Bound", "Holder", "In", "Keys", "Label", "No", "Out", "Probe", "Product", "Yes"
        ])
    );
    Ok(())
}

#[test]
fn should_reference_the_type_a_predicate_narrows_to() -> Result<(), String> {
    let fragment = bind_inline(
        "interface Thing {}\nfunction isThing(value: unknown): value is Thing { return value !== null; }\n",
    )?;

    assert_eq!(
        edges_from(&fragment, "isThing")?,
        type_references(&["Thing"])
    );
    Ok(())
}

#[test]
fn should_reference_the_constraint_of_a_function_type_parameter() -> Result<(), String> {
    let fragment = bind_inline(
        "interface Bound {}\nfunction build<Item extends Bound>(input: Item): Item { return input; }\n",
    )?;

    assert_eq!(edges_from(&fragment, "build")?, type_references(&["Bound"]));
    Ok(())
}

#[test]
fn should_link_a_subclass_to_its_base_by_inheritance_only() -> Result<(), String> {
    let fragment = bind_inline(
        "class Base {}\nclass Derived extends Base implements Unknown { constructor() { super(); } }\n",
    )?;

    assert_eq!(
        edges_from(&fragment, "Derived")?,
        vec![(SmolStr::new("Base"), EdgeKind::Inheritance, Hardness::Hard)]
    );
    Ok(())
}
