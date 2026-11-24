//! Counts of declarations and companions that leave their pass-start file.
//!
//! Test oracle: production scores through `ScoringContext`; this builder is kept
//! so the tests can check the hoisted path against the straightforward one.

use std::collections::{BTreeMap, BTreeSet};

use unlace_ir::{ContainerId, Node, NodeId, NodeKind, Polarity, Snapshot};

/// Counts companions not placed in the immutable pass-start file of their owner.
pub(in crate::analyze) fn companion_separations(
    snapshot: &Snapshot,
    placement: &dyn Fn(u32) -> Option<ContainerId>,
    pass_start_file_by_candidate: &BTreeMap<ContainerId, ContainerId>,
) -> u32 {
    let nodes: BTreeMap<NodeId, &Node> = snapshot
        .ir()
        .nodes
        .iter()
        .map(|node| (node.id, node))
        .collect();
    snapshot
        .ir()
        .affinities
        .iter()
        .copied()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter(|affinity| {
            // a validated snapshot resolves every affinity owner.
            nodes.get(&affinity.owner).is_some_and(|owner| {
                placement(affinity.companion.0)
                    .and_then(|file| pass_start_file_by_candidate.get(&file).copied())
                    != Some(owner.container)
            })
        })
        .count()
        .try_into()
        .unwrap_or(u32::MAX)
}

/// Counts production declarations that leave their pass-start file for a file
/// holding one of their dependencies but none of their consumers. Candidate
/// file ids are mapped back to immutable file identity so moving a whole file
/// between folders contributes zero.
pub(in crate::analyze) fn dependency_only_relocations(
    snapshot: &Snapshot,
    placement: &dyn Fn(u32) -> Option<ContainerId>,
    pass_start_file_by_candidate: &BTreeMap<ContainerId, ContainerId>,
) -> u32 {
    let ir = snapshot.ir();
    let nodes: BTreeMap<u32, &Node> = ir.nodes.iter().map(|node| (node.id.0, node)).collect();
    let mut outgoing_files: BTreeMap<u32, BTreeSet<ContainerId>> = BTreeMap::new();
    let mut incoming_files: BTreeMap<u32, BTreeSet<ContainerId>> = BTreeMap::new();
    // a validated snapshot resolves every edge endpoint.
    let resolved = ir.edges.iter().filter_map(|edge| {
        nodes
            .get(&edge.source.0)
            .zip(nodes.get(&edge.target.0))
            .map(|(source, target)| (edge, source, target))
    });
    for (edge, source, target) in resolved.filter(|(edge, ..)| edge.source != edge.target) {
        outgoing_files
            .entry(edge.source.0)
            .or_default()
            .insert(target.container);
        incoming_files
            .entry(edge.target.0)
            .or_default()
            .insert(source.container);
    }

    ir.nodes
        .iter()
        .filter(|node| {
            node.polarity == Polarity::Production
                && matches!(node.kind, NodeKind::Symbol | NodeKind::Type)
        })
        .filter(|node| {
            let Some(candidate_file) = placement(node.id.0) else {
                return false;
            };
            let Some(destination) = pass_start_file_by_candidate.get(&candidate_file).copied()
            else {
                return false;
            };
            if destination == node.container {
                return false;
            }

            let destination_has_dependency = outgoing_files
                .get(&node.id.0)
                .is_some_and(|files| files.contains(&destination));
            let destination_has_consumer = incoming_files
                .get(&node.id.0)
                .is_some_and(|files| files.contains(&destination));
            destination_has_dependency && !destination_has_consumer
        })
        .count()
        .try_into()
        .unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use unlace_ir::{ContainerId, NodeKind, Polarity, ScopeLevel};

    use super::dependency_only_relocations;
    use crate::analyze::test_support::{container, edge, node, snapshot};

    #[test]
    fn should_count_a_dependency_only_move_only_when_its_candidate_file_maps_back() {
        let snapshot = snapshot(
            vec![
                node(0, "mover", 2, Polarity::Production),
                node(1, "dependency", 3, Polarity::Production),
            ],
            vec![edge(0, 1)],
            vec![
                container(0, "workspace", ScopeLevel::PackageGroup, None),
                container(1, "src", ScopeLevel::Folder, Some(0)),
                container(2, "src/a.ts", ScopeLevel::File, Some(1)),
                container(3, "src/b.ts", ScopeLevel::File, Some(1)),
            ],
        );
        let placement = |id: u32| (id == 0).then_some(ContainerId(7));

        let mapped = BTreeMap::from([(ContainerId(7), ContainerId(3))]);
        assert_eq!(
            dependency_only_relocations(&snapshot, &placement, &mapped),
            1
        );
        assert_eq!(
            dependency_only_relocations(&snapshot, &placement, &BTreeMap::new()),
            0,
            "a candidate file with no pass-start identity relocates nothing"
        );
    }

    #[test]
    fn should_not_count_a_file_body_that_moves_toward_its_dependency_only() {
        let mut file_body = node(0, "mover", 2, Polarity::Production);
        file_body.kind = NodeKind::FileBody;
        let snapshot = snapshot(
            vec![file_body, node(1, "dependency", 3, Polarity::Production)],
            vec![edge(0, 1)],
            vec![
                container(0, "workspace", ScopeLevel::PackageGroup, None),
                container(1, "src", ScopeLevel::Folder, Some(0)),
                container(2, "src/a.ts", ScopeLevel::File, Some(1)),
                container(3, "src/b.ts", ScopeLevel::File, Some(1)),
            ],
        );
        let placement = |id: u32| (id == 0).then_some(ContainerId(7));
        let mapped = BTreeMap::from([(ContainerId(7), ContainerId(3))]);

        assert_eq!(
            dependency_only_relocations(&snapshot, &placement, &mapped),
            0,
            "only declarations count as dependency-only relocations"
        );
    }
}
