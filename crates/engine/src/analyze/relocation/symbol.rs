use std::collections::BTreeMap;

#[cfg(test)]
use unlace_core::visibility::derive_visibility;
use unlace_ir::{ContainerId, Node, NodeKind, Polarity};

use crate::analyze::relocation::{CandidateTree, SYMBOL_MIN_IMPROVEMENT, SYMBOL_SWEEPS};
use crate::analyze::scoring::CycleCounts;
use crate::config::CapacityConfig;

mod admission;
mod fold;
mod guards;
mod inputs;
mod ledger;
mod narrate;
mod outcome;
mod polish;
mod pricing;
mod reach;
mod visibility;

#[cfg(test)]
mod tests;

use admission::Admission;
use inputs::{PassInputs, RelocationPolicy};
use ledger::Ledger;
pub(in crate::analyze) use outcome::SymbolOutcome;
#[cfg(test)]
use outcome::SymbolRelocation;
use pricing::Pricing;
use visibility::VisibilityTracker;

/// One deterministic run of the FIX08 symbol polish over an assembled tree.
///
/// A thin coordinator over four parts, each with one job: the [`Ledger`]
/// owns the trial placement and its bookkeeping, [`Pricing`] nominates and
/// prices, [`Admission`] vetoes, and the [`VisibilityTracker`] holds the
/// visibility floor. The pass itself keeps only the cycle baseline and the
/// methods that must consult several parts in one fixed order. Determinism is
/// structural: symbols sweep in ascending id order, destinations rank by
/// summed two-way priced pull with ties broken toward the lower file id, and
/// every tie elsewhere resolves to staying.
pub(in crate::analyze) struct SymbolPass<'a> {
    assembled: &'a CandidateTree,
    base: &'a BTreeMap<u32, ContainerId>,
    nodes: &'a [Node],
    capacity: CapacityConfig,
    /// Placement overlay, running-best score and the SLOC, occupancy and
    /// relocation ledgers.
    ledger: Ledger<'a>,
    /// Nomination, the objective and the crossing graph.
    pricing: Pricing<'a>,
    /// The structural veto family.
    admission: Admission<'a>,
    /// The working visibility copy and its baseline.
    visibility: VisibilityTracker,
    /// Cycle baseline: relocation may raise neither cyclic dimension.
    cyclic_base: CycleCounts,
}

impl<'a> SymbolPass<'a> {
    fn new_with_policy(inputs: PassInputs<'a>, policy: RelocationPolicy) -> Self {
        let PassInputs {
            assembled,
            nodes,
            capacity,
            ..
        } = inputs;
        let base = &assembled.placement;
        // FIX11: the test-zone tie-cut rides placement into symbol grain. A
        // node's zone is its placed file's mark.
        let touches_zone = |node: u32| -> bool {
            base.get(&node)
                .and_then(|file| assembled.zone_by_file.get(file))
                .copied()
                .unwrap_or(false)
        };
        let mut pass = Self {
            assembled,
            base,
            nodes,
            capacity,
            ledger: Ledger::new(inputs),
            pricing: Pricing::new(inputs, &touches_zone),
            admission: Admission::new(inputs, policy, &touches_zone),
            visibility: VisibilityTracker::new(inputs),
            cyclic_base: CycleCounts::default(),
        };
        pass.ledger.best = pass.pricing.score_with(&pass.ledger.overlay);
        let crossing = pass.pricing.crossing_csr(&pass.ledger);
        pass.cyclic_base = CycleCounts::from_graph(&crossing);
        pass.visibility.vis_base = pass
            .visibility
            .refresh_visibility(nodes, &pass.ledger.overlay);
        pass
    }

    #[cfg(test)]
    fn new(inputs: PassInputs<'a>) -> Self {
        Self::new_with_policy(inputs, RelocationPolicy::default())
    }

    /// Sweeps every symbol in ascending id order, at most
    /// [`SYMBOL_SWEEPS`] times, stopping early once a sweep relocates
    /// nothing.
    fn run(&mut self) {
        for _ in 0..SYMBOL_SWEEPS {
            let mut improved = false;
            for node in self.nodes {
                improved |= self.try_relocate(node);
            }
            if !improved {
                break;
            }
        }
    }

    /// Offers one symbol its strongest-pulling destinations under the full
    /// veto family; records an accepted relocation and returns whether the
    /// sweep made progress.
    #[allow(clippy::too_many_lines)] // reason: one ordered veto ladder; splitting it would scatter the rules that must run in this order
    fn try_relocate(&mut self, node: &Node) -> bool {
        if node.kind == NodeKind::FileBody {
            return false;
        }
        // One move per symbol per candidate (FIX12-C): a second relocation
        // would narrate the same name twice with contradictory destinations.
        if self.ledger.moved.contains(&node.id.0) {
            return false;
        }
        // A detected test declaration stays pinned by its own polarity, not
        // only by its file's zone: a `#[cfg(test)]` helper or `#[test]` case
        // living in a production file would otherwise be free to move.
        if self.admission.policy.pin_test_polarity && node.polarity != Polarity::Production {
            return false;
        }
        if self.admission.policy.start_symbols.contains(&node.id.0) {
            return false;
        }
        let Some(source_file) = self.ledger.effective(node.id.0) else {
            return false;
        };
        if self
            .admission
            .policy
            .forbidden_sources
            .contains(&source_file)
        {
            return false;
        }
        // No empty shells: the origin keeps at least one of the production
        // symbols the assembly placed there. Counted over `native`, so an
        // arrival cannot unlock the drain (FIX12-A).
        if self.ledger.native.get(&source_file).copied().unwrap_or(0) <= 1 {
            return false;
        }
        for destination in self.pricing.nominate(node, source_file) {
            if !self
                .admission
                .admits_nominated(node, source_file, destination, &self.ledger)
            {
                continue;
            }
            // SLOC cap on the destination, priced in production SLOC.
            let destination_sloc = self.ledger.sloc.get(&destination).copied().unwrap_or(0);
            let moving_sloc =
                (node.polarity == Polarity::Production).then_some(node.effective_size);
            if let Some(size) = moving_sloc
                && destination_sloc.saturating_add(size) > self.capacity.file
            {
                continue;
            }

            // Tentatively relocate, then run the structural vetoes.
            let previous = self.place_tentatively(node.id.0, destination);
            let crossing = self.pricing.crossing_csr(&self.ledger);
            let cyclic_now = CycleCounts::from_graph(&crossing);
            if cyclic_now.exceeds(self.cyclic_base) {
                self.revert_relocation(node.id.0, previous, source_file);
                continue;
            }
            let vis_now = self.visibility.count_findings();
            if vis_now > self.visibility.vis_base {
                self.revert_relocation(node.id.0, previous, source_file);
                continue;
            }

            let total = self.pricing.score_with(&self.ledger.overlay);
            // Strict improvement with a real margin: float-dust gains are
            // rejected, not accepted (see SYMBOL_MIN_IMPROVEMENT).
            if self.ledger.best - total > SYMBOL_MIN_IMPROVEMENT {
                let delta = self.ledger.best - total;
                self.ledger.best = total;
                self.cyclic_base = cyclic_now;
                self.visibility.vis_base = vis_now;
                self.ledger
                    .record_relocation(node, source_file, destination, delta, None);
                return true;
            }
            // Rejected: undo the tentative relocation.
            self.revert_relocation(node.id.0, previous, source_file);
        }
        false
    }

    /// Tentatively places one symbol in `destination`: overlays it and moves its
    /// visibility home there. Returns the overlay entry it had before, for
    /// [`Self::revert_relocation`].
    fn place_tentatively(&mut self, node: u32, destination: ContainerId) -> Option<ContainerId> {
        let previous = self.ledger.overlay.insert(node, destination);
        self.visibility.set_visible(node, destination);
        previous
    }

    /// Undoes one tentative relocation: restores the overlay entry the symbol
    /// had before (`previous`) and its visibility home in `source`.
    fn revert_relocation(&mut self, node: u32, previous: Option<ContainerId>, source: ContainerId) {
        self.ledger.undo(node, previous);
        self.visibility.set_visible(node, source);
    }
}

#[cfg(test)]
mod file_capacity_tests {
    use unlace_ir::{Polarity, ScopeLevel, Snapshot};

    use crate::analyze::analyze;
    use crate::analyze::test_support::{container, edge, inherits, node, snapshot};
    use crate::config::AnalyzeConfig;

    /// `s` sits in `a.ts` while both of its subclasses live in `b.ts`, which
    /// holds three production SLOC of its own.
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
                container(3, "a.ts", ScopeLevel::File, Some(1)),
                container(4, "b.ts", ScopeLevel::File, Some(2)),
            ],
        )
    }

    /// The symbols the best greenfield candidate relocates under a file cap.
    /// Overage carries no objective weight, so only the hard cap can stop
    /// the move.
    fn relocated_symbols(file_cap: u32) -> Result<Vec<String>, String> {
        let mut config = AnalyzeConfig::default();
        config.profiles.anchored.candidates = 1;
        config.profiles.greenfield.capacity.file = file_cap;
        config.profiles.greenfield.objective.capacity = 0.0;
        let result = analyze(&misfiled_symbol_snapshot(), &config);
        let candidate = result
            .as_ref()
            .map_err(ToString::to_string)?
            .profiles
            .greenfield
            .as_ref()
            .and_then(|mode| mode.candidates.first())
            .ok_or("no greenfield candidate")?;
        Ok(candidate
            .symbol_moves
            .iter()
            .map(|entry| entry.symbol.clone())
            .collect())
    }

    #[test]
    fn should_refuse_a_symbol_move_that_overfills_its_destination_file() -> Result<(), String> {
        // b.ts holds 3 SLOC; s would make it 4.
        assert_eq!(relocated_symbols(4)?, vec!["s".to_owned()]);
        assert_eq!(relocated_symbols(3)?, Vec::<String>::new());
        Ok(())
    }
}
