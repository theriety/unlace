use std::collections::BTreeMap;

use unlace_core::visibility::PreparedVisibility;
use unlace_ir::{ContainerId, Node};

use crate::analyze::relocation::symbol::inputs::PassInputs;

/// The visibility floor of a symbol pass: a working placement of the node table
/// that follows trial placements, and the finding count a relocation may never
/// raise.
pub(super) struct VisibilityTracker {
    /// The tree, edges and node table the pass derives visibility over, resolved
    /// once; only the working placement changes between queries.
    prepared: PreparedVisibility,
    /// Working container of every node, parallel to the node table. Seeded from
    /// the base placement (the tree the pass runs on, not the node table's
    /// current container ids) and tracking the overlay, so the visibility floor
    /// is derived over trial placements in the same arena the veto reads.
    containers: Vec<ContainerId>,
    /// Position in the node table of each raw node id, `None` when absent.
    position_of: Vec<Option<usize>>,
    /// Visibility baseline: relocation may never raise the finding count.
    pub(super) vis_base: usize,
}

impl VisibilityTracker {
    pub(super) fn new(inputs: PassInputs<'_>) -> Self {
        let PassInputs {
            assembled,
            nodes,
            edges,
            ..
        } = inputs;
        let base = &assembled.placement;
        Self {
            prepared: PreparedVisibility::new(&assembled.tree, nodes, edges),
            containers: nodes
                .iter()
                .map(|node| base.get(&node.id.0).copied().unwrap_or(node.container))
                .collect(),
            position_of: position_table(nodes),
            vis_base: 0,
        }
    }

    /// Applies the whole overlay to the working placement, then counts
    /// findings — the initial baseline path.
    ///
    /// `nodes` must be the table this tracker was built from: containers are
    /// matched to nodes by position.
    pub(super) fn refresh_visibility(
        &mut self,
        nodes: &[Node],
        overlay: &BTreeMap<u32, ContainerId>,
    ) -> usize {
        debug_assert_eq!(nodes.len(), self.containers.len());
        for (node, container) in nodes.iter().zip(&mut self.containers) {
            if let Some(file) = overlay.get(&node.id.0) {
                *container = *file;
            }
        }
        self.count_findings()
    }

    /// Points one node's working container at `container`.
    pub(super) fn set_visible(&mut self, id: u32, container: ContainerId) {
        self.position_of
            .get(id as usize)
            .copied()
            .flatten()
            .and_then(|position| self.containers.get_mut(position))
            .into_iter()
            .for_each(|slot| *slot = container);
    }

    /// Visibility finding count over the working placement as it stands.
    pub(super) fn count_findings(&self) -> usize {
        self.prepared.finding_count(&self.containers)
    }
}

/// Maps each raw node id to its position in `nodes`.
fn position_table(nodes: &[Node]) -> Vec<Option<usize>> {
    let max_id = nodes.iter().map(|node| node.id.0 as usize).max();
    let mut table = vec![None; max_id.map_or(0, |id| id + 1)];
    for (position, node) in nodes.iter().enumerate() {
        table
            .get_mut(node.id.0 as usize)
            .into_iter()
            .for_each(|cell| *cell = Some(position));
    }
    table
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use unlace_core::visibility::PreparedVisibility;
    use unlace_ir::{ContainerId, ContainerTree, Polarity, ScopeLevel};

    use super::{VisibilityTracker, position_table};
    use crate::analyze::test_support::{container, edge, node};

    #[test]
    fn should_count_findings_over_the_overlaid_placement() {
        // `user` in `b.ts` calls the folder-visible `helper` in `a.ts`.
        let tree = ContainerTree::new(vec![
            container(0, "app", ScopeLevel::PackageGroup, None),
            container(1, "src", ScopeLevel::Folder, Some(0)),
            container(2, "src/a.ts", ScopeLevel::File, Some(1)),
            container(3, "src/b.ts", ScopeLevel::File, Some(1)),
        ]);
        let mut helper = node(0, "helper", 2, Polarity::Production);
        helper.visibility = ScopeLevel::Folder;
        let nodes = [helper, node(1, "user", 3, Polarity::Production)];
        let edges = [edge(1, 0)];
        let mut tracker = VisibilityTracker {
            prepared: PreparedVisibility::new(&tree, &nodes, &edges),
            containers: vec![ContainerId(2), ContainerId(3)],
            position_of: position_table(&nodes),
            vis_base: 0,
        };
        let before = tracker.count_findings();

        let after = tracker.refresh_visibility(&nodes, &BTreeMap::from([(0, ContainerId(3))]));

        // beside its only user the helper's folder visibility is over-exported.
        assert_eq!(tracker.containers, vec![ContainerId(3), ContainerId(3)]);
        assert_eq!((before, after), (0, 1));
    }
}
