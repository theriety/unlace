//! Objective evaluation and the J(T)-polish pass.

use std::collections::BTreeMap;

use unlace_core::cluster::{ClusterId, Partition};

#[cfg(test)]
use crate::analyze::relocation::mirror::PolishEvidence;
use crate::analyze::relocation::{CandidateTree, POLISH_SWEEPS, POLISH_TARGETS, PipelineSolver};
use crate::analyze::scoring::{CycleCounts, ScoringContext};

impl PipelineSolver<'_> {
    /// Scores the five-level layout `parts` induces under this mode's
    /// coefficients.
    pub(in crate::analyze) fn evaluate(&self, parts: &Partition) -> f64 {
        let assembled = self.assemble(parts);
        self.evaluate_assembled(&assembled)
    }

    #[cfg(test)]
    pub(in crate::analyze) fn evaluate_with_polish_evidence(
        &self,
        parts: &Partition,
        evidence: &PolishEvidence,
    ) -> f64 {
        let assembled = self.assemble_with_polish_evidence(parts, evidence);
        self.evaluate_assembled(&assembled)
    }

    /// The scoring context of one assembled tree: its tree-dependent facts
    /// resolved once, so any number of placements can be priced against it.
    pub(in crate::analyze::relocation) fn scoring_context<'s>(
        &'s self,
        assembled: &CandidateTree,
    ) -> ScoringContext<'s> {
        ScoringContext::new(
            &self.scoring,
            &assembled.tree,
            &assembled.namespace_by_file,
            &assembled.key_by_id,
            &assembled.pass_start_file_by_candidate,
            &self.capacity,
        )
    }

    fn evaluate_assembled(&self, assembled: &CandidateTree) -> f64 {
        let placement = |id: u32| assembled.placement.get(&id).copied();
        // the assembly's placement maps every node to its current file's
        // candidate id, so its move distance is pure file-grain movement.
        self.scoring_context(assembled)
            .score_placement(&placement, &self.coefficients, &self.weights)
            .total
    }

    /// The J(T)-polish pass: sweeps every file SCC in deterministic order and
    /// greedily relocates it to the strongest-pulling folder whenever the move
    /// strictly lowers the full objective. Physical immediate-entry capacity
    /// (direct files plus direct child directories) and quotient cyclicity stay
    /// hard vetoes, never penalties — but the cyclicity veto is relative, not
    /// absolute: a move is barred when it *grows* either the number of folders
    /// caught in quotient cycles or the edges held inside those cycles, never
    /// for cyclicity the current layout already has. Misplaced files routinely
    /// entangle real folder graphs in cycles no single move can dissolve; an
    /// absolute veto would price every move at infinity on such a base and
    /// freeze the pass wholesale. On an acyclic base the two vetoes agree.
    /// At most [`POLISH_SWEEPS`] passes, stopping early once a sweep applies
    /// no move. Returns the final score so `solve` never re-evaluates.
    pub(in crate::analyze) fn polish(&self, parts: &mut Partition) -> f64 {
        let mut best = self.evaluate(parts);
        let initial_quotient = parts.quotient(&self.condensation.dag);
        let mut cyclic_base = CycleCounts::from_graph(&initial_quotient);
        for _ in 0..POLISH_SWEEPS {
            let mut improved = false;
            for scc in 0..self.condensation.members.len() {
                let scc32 = u32::try_from(scc).unwrap_or(u32::MAX);
                let pinned = self.pinned_scc.get(scc).copied().unwrap_or(false);
                let Some(source) = parts.cluster_of(scc32).filter(|_| !pinned) else {
                    continue;
                };
                for target in self.pull_targets(parts, scc32, source) {
                    if !self.relocation_identity.permits_join(parts, scc32, target) {
                        continue;
                    }
                    // FIX05 (WS-D anchored-inversion): a bridge is not a member of
                    // the thing it bridges. When an SCC's priced edges reach a
                    // folder besides the pair (current, target) — main.py importing
                    // three features, pipeline.py bridging billing and telemetry —
                    // absorbing it into one side strands the rest of its boundary,
                    // yet every locally-scored statistic of the absorber improves:
                    // the adopted edges drop to folder height while the abandoned
                    // ones keep whatever height they already had. The objective
                    // alone therefore ratifies the absorption and greenfield
                    // out-churns anchored, inverting the product promise (the
                    // eval corpus's inversion witness). The veto is structural,
                    // not scored: it fires before evaluation, needs no reference
                    // to the current layout, and so binds both modes equally —
                    // the FIX04 pattern of enforcing contract intent where
                    // admission cannot see it. Zero-priced edges nominate nothing
                    // here either: they never bind placement.
                    if self.absorbs_a_foreign_anchor(parts, scc32, source, target) {
                        continue;
                    }
                    // FIX05 (companion veto): the mirror image of bridge
                    // absorption. An SCC whose current folder pulls at least as
                    // hard as the destination is being torn from measured
                    // company for speculative proximity — the channel that
                    // survived the bridge veto: with the facade unabsorbable,
                    // greedy polish instead walked the feature members out of
                    // their real directories toward it, one transiently cheap
                    // step at a time. Relocation is honest only when the
                    // destination out-pulls what would be stranded (the
                    // satellite joining its sole anchor); a tie resolves to
                    // staying, because folders are reality until priced
                    // evidence says otherwise.
                    if self.strands_a_comparable_anchor(parts, scc32, source, target) {
                        continue;
                    }
                    // FIX05 (third veto): the synthetic bucket is not a place.
                    // `workspace` is the fallback name for files whose real
                    // directory is the project root — an absence of structure,
                    // not a structure. Once the first two vetoes sealed the
                    // feature folders, greedy polish found the remaining exit:
                    // feature members fleeing their real directories INTO the
                    // bucket, because sitting beside the unabsorbable hub
                    // cheapens their hub edges while the bucket prices nothing
                    // back. That flight is the collapse defect itself (real
                    // directories swallowed by an invented container). A file
                    // with priced company in its own folder therefore may not
                    // relocate into the bucket at all; only files reality left
                    // loose belong there, and they are already home.
                    if self.flees_into_the_synthetic_bucket(parts, scc32, source, target) {
                        continue;
                    }
                    if !self.permits_physical_capacity(parts, scc32, target)
                        || !parts.move_node(scc32, target)
                    {
                        continue;
                    }
                    let tentative_quotient = parts.quotient(&self.condensation.dag);
                    let cyclic_now = CycleCounts::from_graph(&tentative_quotient);
                    let total = if cyclic_now.exceeds(cyclic_base) {
                        f64::INFINITY
                    } else {
                        self.evaluate(parts)
                    };
                    if total < best {
                        best = total;
                        cyclic_base = cyclic_now;
                        improved = true;
                        break;
                    }
                    parts.move_node(scc32, source);
                }
            }
            if !improved {
                break;
            }
        }
        best
    }

    /// Ranks the folders pulling hardest on `scc` — summed edge weight over both
    /// directions — and returns up to [`POLISH_TARGETS`] of them, strongest
    /// first, ties broken by the lower cluster id. Zero-priced edges nominate
    /// nothing: they never bind placement (the FIX04 doctrine), so a folder
    /// connected only through re-exports is never offered as a move target.
    pub(in crate::analyze) fn pull_targets(
        &self,
        parts: &Partition,
        scc: u32,
        source: ClusterId,
    ) -> Vec<ClusterId> {
        let mut pull: BTreeMap<ClusterId, f64> = BTreeMap::new();
        for graph in [&self.condensation.dag, &self.reverse_dag] {
            let weights = graph.weights(scc);
            for (slot, &neighbour) in graph.neighbors(scc).iter().enumerate() {
                let Some(cluster) = parts
                    .cluster_of(neighbour)
                    .filter(|&cluster| cluster != source)
                else {
                    continue;
                };
                let weight = weights.get(slot).copied().unwrap_or(0.0);
                if weight <= 0.0 {
                    // FIX04 doctrine: a zero-priced edge never binds placement.
                    continue;
                }
                *pull.entry(cluster).or_insert(0.0) += f64::from(weight);
            }
        }
        let mut ranked: Vec<(ClusterId, f64)> = pull.into_iter().collect();
        ranked.sort_by(|left, right| {
            right
                .1
                .partial_cmp(&left.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(left.0.cmp(&right.0))
        });
        ranked
            .into_iter()
            .take(POLISH_TARGETS)
            .map(|(cluster, _)| cluster)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use unlace_ir::{Polarity, ScopeLevel, Snapshot};

    use crate::analyze::relocation::PipelineSolver;
    use crate::analyze::test_policy::TestPolicy;
    use crate::analyze::test_support::{container, edge, node, snapshot, synthetic_container};
    use crate::config::AnalyzeConfig;

    /// `feat/x.ts` calls its folder-mate `feat/y.ts` once and every symbol of
    /// the root files `main.ts` and `util.ts`; the root files sit in the
    /// synthetic `workspace` bucket, bonded to each other three times over.
    fn bucket_flight_snapshot() -> Snapshot {
        snapshot(
            vec![
                node(0, "x", 5, Polarity::Production),
                node(1, "y", 6, Polarity::Production),
                node(2, "m0", 3, Polarity::Production),
                node(3, "m1", 3, Polarity::Production),
                node(4, "m2", 3, Polarity::Production),
                node(5, "u0", 4, Polarity::Production),
                node(6, "u1", 4, Polarity::Production),
                node(7, "u2", 4, Polarity::Production),
            ],
            vec![
                edge(0, 1),
                edge(0, 2),
                edge(0, 3),
                edge(0, 4),
                edge(0, 5),
                edge(0, 6),
                edge(0, 7),
                edge(2, 5),
                edge(3, 6),
                edge(4, 7),
            ],
            vec![
                container(0, "app", ScopeLevel::PackageGroup, None),
                synthetic_container(1, "workspace", ScopeLevel::Folder, Some(0)),
                container(2, "feat", ScopeLevel::Folder, Some(0)),
                container(3, "main.ts", ScopeLevel::File, Some(1)),
                container(4, "util.ts", ScopeLevel::File, Some(1)),
                container(5, "feat/x.ts", ScopeLevel::File, Some(2)),
                container(6, "feat/y.ts", ScopeLevel::File, Some(2)),
            ],
        )
    }

    #[test]
    fn should_keep_a_file_with_priced_company_out_of_the_synthetic_bucket() {
        let snapshot = bucket_flight_snapshot();
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
        let scc_of = |container: u32| {
            solver
                .index_of
                .get(&container)
                .and_then(|vertex| solver.condensation.membership.get(*vertex as usize))
                .map_or(u32::MAX, |scc| scc.0)
        };
        let (x, main) = (scc_of(5), scc_of(3));
        let mut parts = solver.real_partition.clone();
        let feat = parts.cluster_of(x);
        let bucket = parts.cluster_of(main);
        // the bucket out-pulls x's own folder, so only the flight veto holds x.
        assert!(
            feat.zip(bucket).is_some_and(|(feat, bucket)| {
                solver.pull_targets(&parts, x, feat).first() == Some(&bucket)
                    && !solver.absorbs_a_foreign_anchor(&parts, x, feat, bucket)
                    && !solver.strands_a_comparable_anchor(&parts, x, feat, bucket)
            }),
            "the bucket must be x's strongest unvetoed pull"
        );

        solver.polish(&mut parts);

        assert_eq!(parts.cluster_of(x), feat);
    }
}
