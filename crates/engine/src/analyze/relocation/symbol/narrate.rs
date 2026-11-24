use std::collections::{BTreeMap, BTreeSet};

use unlace_ir::{ContainerId, Node, ScopeLevel};

use crate::analyze::relocation::{CandidateTree, PipelineSolver};
use crate::result::{SymbolKind, SymbolMove};

use super::outcome::SymbolOutcome;

impl PipelineSolver<'_> {
    /// Narrates the accepted symbol relocations as scored [`SymbolMove`] DTOs.
    ///
    /// `from_path` reads the current tree and `to_path` the assembled candidate.
    /// `broken_imports` counts distinct external callers or callees to repoint.
    pub(in crate::analyze::relocation) fn symbol_narrate(
        &self,
        assembled: &CandidateTree,
        symbols: &SymbolOutcome,
    ) -> Vec<SymbolMove> {
        if symbols.relocations.is_empty() {
            return Vec::new();
        }
        let ir = self.snapshot.ir();
        let base = &assembled.placement;
        let effective = |id: u32| -> Option<ContainerId> {
            symbols
                .overlay
                .get(&id)
                .copied()
                .or_else(|| base.get(&id).copied())
        };
        let current_name: BTreeMap<u32, &str> = ir
            .containers
            .containers()
            .iter()
            .map(|container| (container.id.0, container.name.as_str()))
            .collect();
        let candidate_name: BTreeMap<u32, &str> = assembled
            .tree
            .containers()
            .iter()
            .filter(|container| container.level == ScopeLevel::File)
            .map(|container| (container.id.0, container.name.as_str()))
            .collect();
        let kind_of = |node: &Node| match node.kind {
            unlace_ir::NodeKind::Symbol => SymbolKind::Symbol,
            unlace_ir::NodeKind::Type => SymbolKind::Type,
            unlace_ir::NodeKind::FileBody => {
                unreachable!("file-body nodes are immobile and cannot appear in symbol narration")
            }
        };

        let by_node: BTreeMap<u32, &Node> = ir.nodes.iter().map(|node| (node.id.0, node)).collect();
        let mut moves = Vec::with_capacity(symbols.relocations.len());
        for relocation in &symbols.relocations {
            let Some(symbol) = by_node.get(&relocation.node).copied() else {
                continue;
            };
            let Some(&home) = base.get(&relocation.node) else {
                continue;
            };
            debug_assert_eq!(
                home, relocation.from_file,
                "origin file is the symbol's base placement"
            );
            let mut severed: BTreeSet<ContainerId> = BTreeSet::new();
            for edge in &ir.edges {
                let neighbour = if edge.source.0 == relocation.node {
                    edge.target.0
                } else if edge.target.0 == relocation.node {
                    edge.source.0
                } else {
                    continue;
                };
                let Some(place) = effective(neighbour) else {
                    continue;
                };
                if place == relocation.to_file {
                    continue;
                }
                severed.insert(place);
            }
            moves.push(SymbolMove {
                symbol: symbol.name.to_string(),
                display: symbol.display_name.as_ref().map(ToString::to_string),
                kind: kind_of(symbol),
                from_path: current_name
                    .get(&symbol.container.0)
                    .copied()
                    .unwrap_or_default()
                    .to_owned(),
                to_path: candidate_name
                    .get(&relocation.to_file.0)
                    .copied()
                    .unwrap_or_default()
                    .to_owned(),
                delta: -relocation.delta,
                broken_imports: u32::try_from(severed.len()).unwrap_or(u32::MAX),
            });
        }
        moves
    }
}

#[cfg(test)]
mod tests {
    use unlace_ir::{NodeKind, Polarity, ScopeLevel, Snapshot};

    use crate::analyze::relocation::symbol::outcome::{SymbolOutcome, SymbolRelocation};
    use crate::analyze::relocation::{CandidateTree, PipelineSolver};
    use crate::analyze::test_policy::TestPolicy;
    use crate::analyze::test_support::{container, edge, node, snapshot};
    use crate::config::AnalyzeConfig;
    use crate::result::SymbolMove;

    /// `shared_step` (node 0) leaves `source.ts`, where `source_consumer`
    /// (node 1) still calls it, for `destination.ts`, which holds the
    /// dependency it calls (node 2). Node 3 is `source.ts`'s file body.
    fn relocation_snapshot() -> Snapshot {
        let mut body = node(3, "body", 2, Polarity::Production);
        body.kind = NodeKind::FileBody;
        snapshot(
            vec![
                node(0, "shared_step", 2, Polarity::Production),
                node(1, "source_consumer", 2, Polarity::Production),
                node(2, "destination_dependency", 3, Polarity::Production),
                body,
            ],
            vec![edge(1, 0), edge(0, 2)],
            vec![
                container(0, "workspace", ScopeLevel::PackageGroup, None),
                container(1, "area", ScopeLevel::Folder, Some(0)),
                container(2, "area/source.ts", ScopeLevel::File, Some(1)),
                container(3, "area/destination.ts", ScopeLevel::File, Some(1)),
            ],
        )
    }

    /// Narrates `node` relocating from `source.ts` to `destination.ts` over
    /// the assembled tree, after `prune` edits that tree's placement.
    fn narrate_relocation(node: u32, prune: &dyn Fn(&mut CandidateTree)) -> Vec<SymbolMove> {
        let snapshot = relocation_snapshot();
        let tests = TestPolicy::defaults();
        let config = AnalyzeConfig::default();
        let profile = &config.profiles.greenfield;
        let solver = PipelineSolver::new(
            &snapshot,
            profile,
            profile.objective.coefficients(),
            false,
            &tests,
        );
        let mut assembled = solver.assemble(&solver.real_partition);
        let files = assembled
            .placement
            .get(&0)
            .copied()
            .zip(assembled.placement.get(&2).copied());
        prune(&mut assembled);
        files.map_or_else(Vec::new, |(from_file, to_file)| {
            let outcome = SymbolOutcome {
                overlay: [(node, to_file)].into_iter().collect(),
                relocations: vec![SymbolRelocation {
                    node,
                    from_file,
                    to_file,
                    delta: 1.0,
                    fold: None,
                }],
                total: 0.0,
            };
            solver.symbol_narrate(&assembled, &outcome)
        })
    }

    fn keep_all(_: &mut CandidateTree) {}

    /// Gives the unknown node 99 `shared_step`'s base placement, so only the
    /// snapshot lookup can tell it apart.
    fn place_unknown_node_beside_shared_step(tree: &mut CandidateTree) {
        tree.placement
            .extend(tree.placement.get(&0).map(|&home| (99, home)));
    }

    fn broken_imports(moves: &[SymbolMove]) -> Vec<(String, u32)> {
        moves
            .iter()
            .map(|entry| (entry.symbol.clone(), entry.broken_imports))
            .collect()
    }

    #[test]
    fn should_narrate_nothing_for_a_relocated_node_absent_from_the_snapshot() {
        assert_eq!(
            broken_imports(&narrate_relocation(0, &keep_all)),
            vec![("shared_step".to_owned(), 1)]
        );
        assert!(narrate_relocation(99, &place_unknown_node_beside_shared_step).is_empty());
    }

    #[test]
    fn should_narrate_nothing_for_a_relocated_node_with_no_base_placement() {
        let moves = narrate_relocation(0, &|tree: &mut CandidateTree| {
            tree.placement.remove(&0);
        });

        assert!(moves.is_empty());
    }

    #[test]
    fn should_not_count_an_unplaced_neighbour_as_an_import_to_repoint() {
        let moves = narrate_relocation(0, &|tree: &mut CandidateTree| {
            tree.placement.remove(&1);
        });

        assert_eq!(broken_imports(&moves), vec![("shared_step".to_owned(), 0)]);
    }

    #[test]
    #[should_panic(expected = "file-body nodes are immobile")]
    fn should_refuse_to_narrate_a_file_body_relocation() {
        narrate_relocation(3, &keep_all);
    }
}
