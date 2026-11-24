//! Builds the reported candidate for a solved partition.

use std::collections::BTreeMap;

use unlace_core::cluster::Partition;
use unlace_core::diversify::SolvedCandidate;
use unlace_ir::{ContainerId, ContainerTree, Node};

use crate::analyze::relocation::mirror::PolishEvidence;
use crate::analyze::relocation::{CandidateTree, PipelineSolver, symbol};
use crate::analyze::rendering::render_tree;
use crate::analyze::scoring::{score_current_with_affinity, score_current_with_overlay};
use crate::error::UnlaceError;
use crate::narrate::narrate_repository_relative;
use crate::result::{Candidate, ConditionalSplit, ScoreBreakdown};

impl PipelineSolver<'_> {
    /// Attaches solver-owned follower outcomes to their primary source moves.
    pub(super) fn finish(
        &self,
        parts: Partition,
        total: f64,
        evidence: PolishEvidence,
    ) -> SolvedCandidate<PolishEvidence> {
        if let Some(identity) = &self.identity
            && identity == &parts
        {
            // A faithful partition is priced on the current tree with its
            // symbol overlay applied (`symbol_polish_with_polish_evidence`), so
            // an improving symbol polish survives as an offer; without one it
            // collapses onto the "change nothing" identity entry.
            let entry = self.identity_entry(identity);
            if total >= entry.score {
                return entry;
            }
        }
        SolvedCandidate {
            partition: parts,
            score: total,
            evidence,
        }
    }

    /// The identity pool entry: the "change nothing" layout scored on the actual
    /// current tree, so the anchored pool always contains the current score and
    /// a suggested candidate can never silently lose to it.
    pub(super) fn identity_entry(&self, identity: &Partition) -> SolvedCandidate<PolishEvidence> {
        SolvedCandidate {
            partition: identity.clone(),
            score: score_current_with_affinity(
                self.snapshot,
                &self.coefficients,
                &self.weights,
                &self.capacity,
                self.same_file_symbol,
                self.same_file_type,
            )
            .total,
            evidence: PolishEvidence::default(),
        }
    }

    /// Whether a solved partition is the current layout itself (FIX05).
    ///
    /// The faithful exit keys on the layout being reality, not on the analysis
    /// mode. Gating it on `identity` alone (anchored-only) meant a greenfield
    /// candidate whose partition is byte-for-byte the real directory layout
    /// still went through assemble() — which re-elects upper-level containers
    /// and nests them differently from the current tree — so "change nothing"
    /// rendered as structural moves for every file whose fabricated chain
    /// differed. When relief split an over-capacity folder, the search's start
    /// is no longer reality, so the assemble path stays (the split is exactly
    /// what the proposal must show).
    pub(super) fn is_faithful(&self, partition: &Partition) -> bool {
        self.identity.as_ref() == Some(partition)
            || (self.real_is_identity && &self.real_partition == partition)
    }

    /// The current tree as a [`CandidateTree`]: every file keeps its current
    /// container id and every declaration its current file. A faithful layout
    /// renders as exactly this tree, so its symbol pass runs here, with every
    /// structural veto and every price read from the tree the report shows.
    pub(in crate::analyze) fn current_candidate_tree(&self) -> CandidateTree {
        let ir = self.snapshot.ir();
        let mut pass_start_file_by_candidate = BTreeMap::new();
        let mut zone_by_file = BTreeMap::new();
        let mut namespace_by_file = BTreeMap::new();
        let mut package_by_file = BTreeMap::new();
        for (vertex, file) in self.files.iter().enumerate() {
            let id = ContainerId(file.container);
            pass_start_file_by_candidate.insert(id, id);
            zone_by_file.insert(id, self.test_zone.get(vertex).copied().unwrap_or(false));
            namespace_by_file.insert(id, file.namespace.clone());
            package_by_file.insert(id, file.home.package.clone());
        }
        let placement = ir
            .nodes
            .iter()
            .filter(|node| pass_start_file_by_candidate.contains_key(&node.container))
            .map(|node| (node.id.0, node.container))
            .collect();
        CandidateTree {
            tree: ir.containers.clone(),
            placement,
            pass_start_file_by_candidate,
            zone_by_file,
            namespace_by_file,
            package_by_file,
            key_by_id: BTreeMap::new(),
        }
    }

    /// Prices the current tree with `overlay` (current file ids) applied.
    pub(super) fn score_faithful(
        &self,
        overlay: &BTreeMap<u32, ContainerId>,
    ) -> unlace_core::score::ScoreBreakdown {
        score_current_with_overlay(
            self.snapshot,
            &self.coefficients,
            &self.weights,
            &self.capacity,
            self.same_file_symbol,
            self.same_file_type,
            overlay,
        )
    }

    /// Builds the candidate for a faithful layout: the current tree verbatim,
    /// no file moves, and whatever symbol relocations the symbol polish
    /// accepted on it, priced on the current tree. With no accepted symbol
    /// move this is "change nothing" at exactly the current score.
    fn build_faithful_candidate(
        &self,
        current_tree: &ContainerTree,
        solved: &SolvedCandidate<PolishEvidence>,
        index: u32,
        splits: &[ConditionalSplit],
    ) -> Result<Candidate, UnlaceError> {
        let nodes = &self.snapshot.ir().nodes;
        let current = self.current_candidate_tree();
        let symbols = self.symbol_polish_with_polish_evidence(&solved.partition, &solved.evidence);
        // coverage: unreachable, the symbol pass runs on this same current tree and `Admission::admits` refuses every move outside its render namespace
        ensure_namespaces_preserved(&symbols, &current)?;
        let overlay = &symbols.overlay;
        let breakdown = self.score_faithful(overlay);
        let placement_of =
            |node: &Node| Some(overlay.get(&node.id.0).copied().unwrap_or(node.container));
        // coverage: unreachable, `current_tree` is the snapshot's own tree, which `analyze_inner` already rendered (so it has a root) before any candidate is built
        let node = render_tree(current_tree, nodes, &placement_of, &BTreeMap::new())?;
        Ok(Candidate {
            index,
            score: breakdown.total,
            score_breakdown: ScoreBreakdown::from(breakdown),
            improvement: 0.0,
            tree: node,
            conditional_splits: splits.to_vec(),
            delta_narration: Vec::new(),
            symbol_moves: self.symbol_narrate(&current, &symbols),
            capacity_remainder: None,
        })
    }

    /// Builds one DTO [`Candidate`] from a solved partition.
    ///
    /// A faithful partition keeps the current tree verbatim — zero file moves,
    /// never re-derived through assembly — and carries only its accepted
    /// symbol relocations, priced on that tree exactly as ranking priced them.
    /// Every other partition is assembled, rescored under this mode's
    /// coefficients, and narrated against the current layout.
    ///
    /// # Errors
    ///
    /// Returns [`UnlaceError::SnapshotInvalid`] if the tree cannot be rendered.
    pub(in crate::analyze) fn build_candidate(
        &self,
        current_tree: &ContainerTree,
        solved: &SolvedCandidate<PolishEvidence>,
        index: u32,
        splits: &[ConditionalSplit],
    ) -> Result<Candidate, UnlaceError> {
        let nodes = &self.snapshot.ir().nodes;
        if self.is_faithful(&solved.partition) {
            return self.build_faithful_candidate(current_tree, solved, index, splits);
        }

        let assembled = self.assemble_with_polish_evidence(&solved.partition, &solved.evidence);
        // FIX08: re-run the deterministic symbol pass on this exact partition.
        // `solve` already priced its result into the ranking score, so the DTO
        // score here matches what ranked this candidate by construction.
        let symbols = self.symbol_polish_with_polish_evidence(&solved.partition, &solved.evidence);
        // coverage: unreachable, the symbol pass runs on this same assembled tree and `Admission::admits` refuses every move outside its render namespace
        ensure_namespaces_preserved(&symbols, &assembled)?;
        let merged = |id: u32| {
            symbols
                .overlay
                .get(&id)
                .copied()
                .or_else(|| assembled.placement.get(&id).copied())
        };
        let breakdown = self.scoring_context(&assembled).score_placement(
            &merged,
            &self.coefficients,
            &self.weights,
        );
        let placement_of = |node: &Node| merged(node.id.0);
        // coverage: unreachable, a non-faithful partition places at least one file (with no files every partition equals the real one, which is faithful), so `intern_upper_levels` pushes a root package group
        let node = render_tree(&assembled.tree, nodes, &placement_of, &assembled.key_by_id)?;
        let mut delta = narrate_repository_relative(current_tree, &assembled.tree, &self.facts);
        Self::attach_test_mirrors(&mut delta, &solved.evidence);
        let symbol_moves = self.symbol_narrate(&assembled, &symbols);

        Ok(Candidate {
            index,
            score: breakdown.total,
            score_breakdown: ScoreBreakdown::from(breakdown),
            improvement: 0.0,
            tree: node,
            conditional_splits: splits.to_vec(),
            delta_narration: delta,
            symbol_moves,
            capacity_remainder: None,
        })
    }
}

/// Fails when the symbol pass moved a declaration across a pass-start render
/// namespace of `tree`.
fn ensure_namespaces_preserved(
    symbols: &symbol::SymbolOutcome,
    tree: &CandidateTree,
) -> Result<(), UnlaceError> {
    if symbols.preserves_namespaces(tree) {
        return Ok(());
    }
    Err(UnlaceError::SnapshotInvalid {
        source: unlace_ir::SnapshotError::Serialization {
            reason: "symbol relocation crossed a pass-start render namespace".to_owned(),
        },
    })
}

#[cfg(test)]
mod tests {
    use smol_str::SmolStr;
    use unlace_ir::{Polarity, ScopeLevel, Snapshot};

    use super::ensure_namespaces_preserved;
    use crate::analyze::relocation::PipelineSolver;
    use crate::analyze::relocation::mirror::PolishEvidence;
    use crate::analyze::test_policy::TestPolicy;
    use crate::analyze::test_support::{container, edge, inherits, node, snapshot};
    use crate::config::AnalyzeConfig;
    use crate::error::UnlaceError;

    /// `s` sits in `keep/a.ts` while both of its subclasses live in
    /// `sink/b.ts`, so the symbol pass relocates it there.
    fn misfiled_symbol_snapshot() -> Snapshot {
        snapshot(
            vec![
                node(0, "s", 3, Polarity::Production),
                node(1, "mate", 3, Polarity::Production),
                node(2, "c1", 4, Polarity::Production),
                node(3, "c2", 4, Polarity::Production),
                node(4, "base", 4, Polarity::Production),
            ],
            vec![inherits(2, 0), inherits(3, 0), edge(4, 2), edge(4, 3)],
            vec![
                container(0, "app", ScopeLevel::PackageGroup, None),
                container(1, "keep", ScopeLevel::Folder, Some(0)),
                container(2, "sink", ScopeLevel::Folder, Some(0)),
                container(3, "keep/a.ts", ScopeLevel::File, Some(1)),
                container(4, "sink/b.ts", ScopeLevel::File, Some(2)),
            ],
        )
    }

    #[test]
    fn should_fail_when_an_accepted_symbol_move_crosses_a_render_namespace() {
        let snapshot = misfiled_symbol_snapshot();
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
        let outcome = solver
            .symbol_polish_with_polish_evidence(&solver.real_partition, &PolishEvidence::default());
        let mut tree = solver.current_candidate_tree();
        assert!(
            !outcome.overlay.is_empty(),
            "the fixture must relocate a symbol"
        );
        assert!(ensure_namespaces_preserved(&outcome, &tree).is_ok());

        for destination in outcome.overlay.values() {
            tree.namespace_by_file
                .insert(*destination, SmolStr::new("elsewhere"));
        }

        assert!(matches!(
            ensure_namespaces_preserved(&outcome, &tree),
            Err(UnlaceError::SnapshotInvalid { .. })
        ));
    }
}
