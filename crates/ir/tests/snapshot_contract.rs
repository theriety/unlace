//! The snapshot contract as a downstream crate sees it: an adapter ships an
//! intermediate representation as JSON, and the solver side assembles it into a
//! validated, content-addressed snapshot using only the public API.

use smol_str::SmolStr;
use unlace_ir::{
    Container, ContainerId, ContainerTree, Edge, EdgeKind, Hardness, IntermediateRepresentation,
    Node, NodeId, NodeKind, Polarity, ScopeLevel, Snapshot, SnapshotError,
};

fn root() -> Container {
    Container {
        id: ContainerId(0),
        name: SmolStr::new("root"),
        level: ScopeLevel::File,
        parent: None,
        synthetic: false,
        label_keyed: false,
    }
}

fn node(id: u32, name: &str) -> Node {
    Node {
        public_api: false,
        advice_exempt: false,
        id: NodeId(id),
        name: SmolStr::new(name),
        kind: NodeKind::Symbol,
        polarity: Polarity::Production,
        container: ContainerId(0),
        visibility: ScopeLevel::File,
        effective_size: 1,
        re_export: false,
        display_name: None,
        test_source: false,
        owner: None,
    }
}

fn call(source: u32, target: u32) -> Edge {
    Edge {
        source: NodeId(source),
        target: NodeId(target),
        kind: EdgeKind::Call,
        hardness: Hardness::Hard,
        confidence: 1.0,
        fan_out: false,
    }
}

fn representation(nodes: Vec<Node>, edges: Vec<Edge>) -> IntermediateRepresentation {
    IntermediateRepresentation::new(nodes, edges, ContainerTree::new(vec![root()]))
}

fn node_order(snapshot: &Snapshot) -> Vec<u32> {
    snapshot.ir().nodes.iter().map(|node| node.id.0).collect()
}

#[test]
fn should_assemble_an_identical_snapshot_after_a_json_round_trip() {
    let shipped = representation(
        vec![node(2, "c"), node(0, "a"), node(1, "b")],
        vec![call(1, 2), call(0, 1)],
    );

    let wire = serde_json::to_string(&shipped).ok();
    let received = wire
        .as_deref()
        .and_then(|text| serde_json::from_str::<IntermediateRepresentation>(text).ok());
    let direct = Snapshot::assemble(shipped.clone());
    let from_wire = received.map(Snapshot::assemble);

    assert_eq!(from_wire, Some(direct.clone()));
    assert_eq!(direct.as_ref().map(node_order), Ok(vec![0, 1, 2]));
}

#[test]
fn should_reject_a_shipped_edge_that_references_an_unknown_node() {
    let shipped = representation(vec![node(0, "a")], vec![call(0, 7)]);

    let assembled = Snapshot::assemble(shipped);

    assert_eq!(
        assembled,
        Err(SnapshotError::DanglingEdge {
            source_node: 0,
            target_node: 7,
            missing: 7,
        })
    );
}
